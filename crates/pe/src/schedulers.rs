use crossbeam::queue::SegQueue;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering}, Mutex
};

use smallvec::SmallVec;

use crate::{FinishExecFlags, IncarnationStatus, Task, TxIdx, TxStatus, TxVersion, ValidationStatus};

#[derive(Debug)]
pub(crate) struct TransactionsGraph {
    /// The number of transactions in this block.
    block_size: usize,
    /// The number of transactions that have been executed.
    num_done: AtomicUsize,
    /// The queue of transactions to execute.
    transactions_queue: SegQueue<TxIdx>,
    /// The number of transactions each transaction depends on.
    transactions_degree: Vec<AtomicUsize>,
    /// The list of dependent transactions to resume when the
    /// key transaction is re-executed.
    // TODO2: USE Graph or other data structure to store the dependencies
    transactions_dependents: Vec<Mutex<Vec<TxIdx>>>,
}

impl TransactionsGraph {
    pub(crate) fn new(block_size: usize) -> Self {
        Self {
            block_size,
            num_done: AtomicUsize::new(0),
            transactions_queue: SegQueue::new(),
            transactions_degree: (0..block_size).map(|_| AtomicUsize::new(0)).collect(),
            transactions_dependents: (0..block_size).map(|_| Mutex::default()).collect(),
        }
    }

    pub(crate) fn init(&self) {
        for i in 0..self.block_size {
            if self.transactions_degree[i].load(Ordering::Relaxed) == 0 {
                self.transactions_queue.push(i);
            }
        }
    }

    pub(crate) fn add_dependency(&self, tx_idx: TxIdx, blocking_tx_idx: TxIdx) {
        let mut blocking_dependents = index_mutex!(self.transactions_dependents, blocking_tx_idx);
        blocking_dependents.push(tx_idx);
        self.transactions_degree[tx_idx].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn remove_dependency(&self, blocking_tx_idx: TxIdx) {
        let mut blocking_dependents = index_mutex!(self.transactions_dependents, blocking_tx_idx);
        for txid in blocking_dependents.iter() {
            let degree = self.transactions_degree[*txid].fetch_sub(1, Ordering::Relaxed);
            if degree == 0 {
                self.transactions_queue.push(*txid);
            }
        }

        blocking_dependents.clear();
    }

    pub(crate) fn pop(&self) -> Option<TxIdx> {
        if let Some(txid) = self.transactions_queue.pop() {
            self.num_done.fetch_sub(1, Ordering::Relaxed);
            Some(txid)
        } else {
            None
        }
    }

    pub(crate) fn block_size(&self) -> usize {
        self.block_size
    }

    pub(crate) fn size(&self) -> usize {
        self.block_size - self.num_done.load(Ordering::Relaxed)
    }
}

pub trait TaskProvider {
    /// Returns the task ID of the next task to execute, or None if no tasks remain.
    fn next_task(&self) -> Option<usize>;

    /// Marks the task with the given ID as completed.
    fn finish_task(&self, id: usize);

    /// total tasks
    fn num_tasks(&self) -> usize;
}

#[derive(Debug)]
pub struct DAGProvider {
    graph: TransactionsGraph,
}

impl DAGProvider {
    pub(crate) fn new(block_size: usize) -> Self {
        let graph = TransactionsGraph::new(block_size);
        graph.init();

        Self { graph }
    }
}

impl TaskProvider for DAGProvider {
    fn next_task(&self) -> Option<usize> {
        self.graph.pop()
    }

    fn finish_task(&self, id: usize) {
        self.graph.remove_dependency(id);
    }

    fn num_tasks(&self) -> usize {
        self.graph.block_size()
    }
}

#[derive(Debug)]
pub struct NormalProvider {
    /// The number of transactions in this block.
    block_size: usize,
    /// The next transaction to try and execute.
    execution_idx: AtomicUsize,
}

impl NormalProvider {
    pub fn new(block_size: usize) -> Self {
        Self {
            block_size,
            execution_idx: AtomicUsize::new(0),
        }
    }
}

impl TaskProvider for NormalProvider {
    fn next_task(&self) -> Option<usize> {
        let idx = self.execution_idx.fetch_add(1, Ordering::Relaxed);
        if idx < self.block_size {
            Some(idx)
        } else {
            None
        }
    }

    fn finish_task(&self, _id: usize) {}

    fn num_tasks(&self) -> usize {
        self.block_size
    }
}

// TODO2：use one scheduler for both execution and validation
// The parallel executor collaborative scheduler coordinates execution & validation
// tasks among work threads.
//
// To pick a task, threads increment the smaller of the (execution and
// validation) task counters until they find a task that is ready to be
// performed. To redo a task for a transaction, the thread updates the status
// and reduces the corresponding counter to the transaction index if it had a
// larger value.
//
// An incarnation may write to a memory location that was previously
// read by a higher transaction. Thus, when an incarnation finishes, new
// validation tasks are created for higher transactions.
//
// Validation tasks are scheduled optimistically and in parallel. Identifying
// validation failures and aborting incarnations as soon as possible is critical
// for performance, as any incarnation that reads values written by an
// incarnation that aborts also must abort.
// When an incarnation writes only to a subset of memory locations written
// by the previously completed incarnation of the same transaction, we schedule
// validation just for the incarnation. This is sufficient as the whole write
// set of the previous incarnation is marked as ESTIMATE during the abort.
// The abort leads to optimistically creating validation tasks for higher
// transactions. Threads that perform these tasks can already detect validation
// failure due to the ESTIMATE markers on memory locations, instead of waiting
// for a subsequent incarnation to finish.
#[derive(Debug)]
pub(crate) struct ExeScheduler<T: TaskProvider> {
    /// The provider of transactions.
    provider: T,
    /// the queue of execution tasks
    blocking_queue: SegQueue<TxIdx>,
    /// the queue of validation tasks
    validation_queue: SegQueue<TxIdx>,
    /// The most up-to-date incarnation number (initially 0) and
    /// the status of this incarnation.
    // TODO: Consider packing [TxStatus]s into atomics instead of
    // [Mutex] given how small they are.
    // TODO2: use AtomicUsize
    transactions_status: Vec<Mutex<TxStatus>>,
    /// pending validation tasks
    pending_validation: Vec<AtomicBool>,   
    /// The list of dependent transactions to resume when the
    /// key transaction is re-executed.
    // TODO2: USE Graph or other data structure to store the dependencies
    transactions_dependents: Vec<Mutex<SmallVec<[TxIdx; 1]>>>,
    /// The number of validated transactions
    num_validated: AtomicUsize,
}

impl<T: TaskProvider> ExeScheduler<T> {
    pub(crate) fn new(provider: T) -> Self {
        let block_size = provider.num_tasks();
        Self {
            provider,
            blocking_queue: SegQueue::new(),
            validation_queue: SegQueue::new(),
            transactions_status: (0..block_size)
                .map(|_| {
                    Mutex::new(TxStatus {
                        incarnation: 0,
                        status: IncarnationStatus::ReadyToExecute,
                    })
                })
                .collect(),
            pending_validation: (0..block_size).map(|_| AtomicBool::new(false)).collect(),
            transactions_dependents: (0..block_size).map(|_| Mutex::default()).collect(),
            num_validated: AtomicUsize::new(0),
        }
    }

    fn try_execute(&self, tx_idx: TxIdx) -> Option<TxVersion> {
        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        if tx.status == IncarnationStatus::ReadyToExecute {
            tx.status = IncarnationStatus::Executing;
            return Some(TxVersion {
                tx_idx,
                tx_incarnation: tx.incarnation,
            });
        }
        None
    }

    // next_task returns the next task to execute.
    pub(crate) fn next_task(&self) -> Option<Task> {
        if let Some(tx_idx) = self.validation_queue.pop() {
            self.pending_validation[tx_idx].store(false, Ordering::Relaxed);
            return Some(Task::Validation(tx_idx));
        }
        // Try to get the next task from the provider.
        if let Some(tx_idx) = self.provider.next_task() {
            if let Some(tx_version) = self.try_execute(tx_idx) {
                return Some(Task::Execution(tx_version));
            }
        }

        if let Some(tx_idx) = self.blocking_queue.pop() {
            if let Some(tx_version) = self.try_execute(tx_idx) {
                return Some(Task::Execution(tx_version));
            }
        }

        None
    }

    // Add [tx_idx] as a dependent of [blocking_tx_idx] so [tx_idx] is
    // re-executed when the next [blocking_tx_idx] incarnation is executed.
    // Return [false] if we encounter a race condition when [blocking_tx_idx]
    // gets re-executed before the dependency can be added.
    pub(crate) fn add_dependency(&self, tx_idx: TxIdx, blocking_tx_idx: TxIdx) -> bool {
        // This is an important lock to prevent a race condition where the blocking
        // transaction completes re-execution before this dependency can be added.
        let blocking_tx = index_mutex!(self.transactions_status, blocking_tx_idx);
        if matches!(
            blocking_tx.status,
            IncarnationStatus::Executed | IncarnationStatus::Validated
        ) {
            return false;
        }

        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        debug_assert_eq!(tx.status, IncarnationStatus::Executing);
        tx.status = IncarnationStatus::Blocking;

        let mut blocking_dependents = index_mutex!(self.transactions_dependents, blocking_tx_idx);
        blocking_dependents.push(tx_idx);

        true
    }

    fn add_re_execution(&self, tx_idx: TxIdx) {
        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        debug_assert_eq!(tx.status, IncarnationStatus::Aborting);
        tx.status = IncarnationStatus::ReadyToExecute;
        tx.incarnation += 1;
        self.validation_queue.push(tx_idx);
    }

    fn add_blocking_task(&self, tx_idx: TxIdx) {
        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        debug_assert_eq!(tx.status, IncarnationStatus::Blocking);
        tx.status = IncarnationStatus::ReadyToExecute;
        tx.incarnation += 1;
        self.blocking_queue.push(tx_idx);
        
    }

    fn add_validation_task(&self, tx_idx: TxIdx) {
        // If the transaction is already in the validation queue, skip it.
        if self.pending_validation[tx_idx].swap(true, Ordering::Relaxed) {
            return;
        }
        self.validation_queue.push(tx_idx);
    }

    // TODO: add new transaction status: NoReading
    pub(crate) fn finish_execution(
        &self,
        tx_version: TxVersion,
        flags: FinishExecFlags,
        affected_transactions: Vec<TxIdx>,
    ) -> Option<Task> {
        {
            let mut tx = index_mutex!(self.transactions_status, tx_version.tx_idx);
            // If the transaction is aborted, we need to re-execute it.
            if matches!(tx.status, IncarnationStatus::Aborting) {
                //TODO2: use function
                tx.status = IncarnationStatus::Executing;
                tx.incarnation += 1;
                return Some(Task::Execution(TxVersion { tx_idx: tx_version.tx_idx, tx_incarnation: tx.incarnation }));
            }

            self.provider.finish_task(tx_version.tx_idx);
            
            debug_assert_eq!(tx.status, IncarnationStatus::Executing);
            debug_assert_eq!(tx.incarnation, tx_version.tx_incarnation);

            if flags.contains(FinishExecFlags::NeedValidation) {
                tx.status = IncarnationStatus::Executed;
            } else {
                tx.status = IncarnationStatus::Validated;
            }
        }

        // Resume dependent transactions
        let mut dependents = index_mutex!(self.transactions_dependents, tx_version.tx_idx);
        for tx_idx in dependents.drain(..) {
            // TODO2: skip validation
            self.add_blocking_task(tx_idx);
        }
        
        for tx_idx in affected_transactions {
            self.add_validation_task(tx_idx);
        }

        self.num_validated.fetch_add(1, Ordering::Relaxed);

        None
    }

    pub(crate) fn is_finish(&self) -> bool {
        self.num_validated.load(Ordering::Relaxed) == self.provider.num_tasks()
    }

    // When there is a successful abort, schedule the transaction for re-execution
    // and the higher transactions for validation. The re-execution task is returned
    // for the aborted transaction.
    pub(crate) fn finish_validation(&self, tx_idx: TxIdx, aborted: bool) -> Option<Task> {
        let mut tx = index_mutex!(self.transactions_status, tx_idx);
        if aborted {
            if matches!(
                tx.status,
                IncarnationStatus::Validated | IncarnationStatus::Executed
            ) {
                self.num_validated.fetch_sub(1, Ordering::Relaxed);
                //TODO2: use function
                tx.status = IncarnationStatus::Executing;
                tx.incarnation += 1;
                return Some(Task::Execution(TxVersion { tx_idx, tx_incarnation: tx.incarnation }));
            }
            tx.status = IncarnationStatus::Aborting;
            None
        } else {
            tx.status = IncarnationStatus::Validated;
            None
        }
    }
}

#[derive(Debug)]
pub(crate) struct ValidationScheduler {
    /// the queue of reexecution tasks
    task_queue: SegQueue<TxIdx>,
    /// The number of validated transactions
    transactions_status: Vec<Mutex<ValidationStatus>>,
}

impl ValidationScheduler {
    pub(crate) fn new(num_tasks: usize) -> Self {
        Self {
            task_queue: SegQueue::new(),
            transactions_status: (0..num_tasks)
                .map(|_| Mutex::new(ValidationStatus::NotReady))
                .collect(),
        }
    }

    pub(crate) fn next_task(&self) -> Option<TxIdx> {
        if let Some(txid) = self.task_queue.pop() {
            let mut tx_status = index_mutex!(self.transactions_status, txid);
            *tx_status = ValidationStatus::Validated;
            Some(txid)
        } else {
            None
        }
    }

    pub(crate) fn add_tasks(&self, txids: Vec<TxIdx>) {
        for txid in txids {
            let mut tx_status = index_mutex!(self.transactions_status, txid);
            if matches!(
                *tx_status,
                ValidationStatus::NotReady | ValidationStatus::Validated
            ) {
                *tx_status = ValidationStatus::Waiting;
                self.task_queue.push(txid);
            }
        }
    }
}
