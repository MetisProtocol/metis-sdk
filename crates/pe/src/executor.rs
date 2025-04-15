use crate::{
    MemoryEntry, MemoryLocation, MemoryValue, Task, TxIdx, TxVersion,
    mv_memory::{MvMemory, build_mv_memory},
    scheduler::{NormalProvider, Scheduler, TaskProvider},
    vm::{ExecutionError, TxExecutionResult, Vm, VmExecutionError, VmExecutionResult, build_evm},
};
#[cfg(feature = "compiler")]
use std::sync::Arc;
use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{Mutex, OnceLock, mpsc},
    thread,
};

use alloy_evm::EvmEnv;
use alloy_primitives::{TxNonce, U256};
use metis_primitives::{
    Account, AccountInfo, KECCAK_EMPTY, SpecId, Transaction, hash_deterministic,
};
#[cfg(feature = "compiler")]
use metis_vm::ExtCompileWorker;
#[cfg(feature = "compiler")]
use revm::ExecuteEvm;
use revm::{
    context::{TxEnv, result::InvalidTransaction},
    database::CacheDB,
};

use revm::DatabaseRef;

/// Errors when executing a block with the parallel executor.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParallelExecutorError {
    /// Transactions lack information for execution.
    #[error("The transaction data is missing")]
    MissingTransactionData,
    /// Nonce mismatch error including nonce too low and nonce too high errors.
    #[error("Nonce mismatch for tx #{tx_idx}. Expected {executed_nonce}, got {tx_nonce}")]
    NonceMismatch {
        /// Transaction index
        tx_idx: TxIdx,
        /// Nonce from tx (from the very input)
        tx_nonce: TxNonce,
        /// Nonce from state and execution
        executed_nonce: TxNonce,
    },
    #[error("Storage error: {0}")]
    StorageError(String),
    #[error("Execution error")]
    ExecutionError(
        #[source]
        #[from]
        ExecutionError,
    ),
    #[error("Unreachable error")]
    UnreachableError,
}

/// Execution result of a block
pub type ParallelExecutorResult = Result<Vec<TxExecutionResult>, ParallelExecutorError>;

#[derive(Debug)]
enum AbortReason {
    FallbackToSequential,
    ExecutionError(ExecutionError),
}

// TODO: Better implementation
#[derive(Debug)]
struct AsyncDropper<T> {
    sender: mpsc::Sender<T>,
    _handle: thread::JoinHandle<()>,
}

impl<T: Send + 'static> Default for AsyncDropper<T> {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            _handle: std::thread::spawn(move || receiver.into_iter().for_each(drop)),
        }
    }
}

impl<T> AsyncDropper<T> {
    fn drop(&self, t: T) {
        // TODO: Better error handling
        self.sender.send(t).unwrap();
    }
}

// TODO: Add Scheduler to the dropper
// TODO: Port more recyclable resources into here.
/// The main executor struct that executes blocks with Block-STM algorithm.
#[derive(Debug)]
#[cfg_attr(not(feature = "compiler"), derive(Default))]
pub struct ParallelExecutor {
    execution_results: Vec<Mutex<Option<TxExecutionResult>>>,
    abort_reason: OnceLock<AbortReason>,
    dropper: AsyncDropper<(MvMemory, Scheduler<NormalProvider>, Vec<TxEnv>)>,
    /// The compile work shared with different vm instance.
    #[cfg(feature = "compiler")]
    pub worker: Arc<ExtCompileWorker>,
}

#[cfg(feature = "compiler")]
impl Default for ParallelExecutor {
    fn default() -> Self {
        Self {
            execution_results: Default::default(),
            abort_reason: Default::default(),
            dropper: Default::default(),
            worker: Arc::new(ExtCompileWorker::disable()),
        }
    }
}

impl ParallelExecutor {
    /// New a parrallel VM with the compiler feature. The default compiler is an AOT-based one.
    #[cfg(feature = "compiler")]
    pub fn compiler() -> Self {
        Self {
            worker: Arc::new(ExtCompileWorker::aot().expect("compile worker init failed")),
            ..Default::default()
        }
    }
}

impl ParallelExecutor {
    /// Execute an block with the block env and transactions.
    pub fn execute<DB>(
        &mut self,
        db: DB,
        evm_env: EvmEnv,
        txs: Vec<TxEnv>,
        concurrency_level: NonZeroUsize,
    ) -> ParallelExecutorResult
    where
        DB: DatabaseRef + Send + Sync,
    {
        if txs.is_empty() {
            return Ok(Vec::new());
        }

        let block_size = txs.len();
        let task_provider = NormalProvider::new(block_size);
        let scheduler = Scheduler::new(task_provider);

        let mv_memory = build_mv_memory(&evm_env.block_env, &txs);
        let vm = Vm::new(
            &db,
            &mv_memory,
            &evm_env,
            &txs,
            #[cfg(feature = "compiler")]
            self.worker.clone(),
        );

        let additional = block_size.saturating_sub(self.execution_results.len());
        if additional > 0 {
            self.execution_results.reserve(additional);
            for _ in 0..additional {
                self.execution_results.push(Mutex::new(None));
            }
        }

        // TODO: Better thread handling
        thread::scope(|scope| {
            for _ in 0..concurrency_level.into() {
                scope.spawn(|| {
                    let mut task = scheduler.next_task();
                    while self.abort_reason.get().is_none() {
                        task = match task {
                            Some(Task::Execution(tx_version)) => {
                                self.try_execute(&vm, &scheduler, tx_version)
                            }
                            Some(Task::Validation(tx_idx)) => {
                                try_validate(&mv_memory, &scheduler, tx_idx)
                            }
                            None => None,
                        };

                        if task.is_some() {
                            continue;
                        }

                        task = scheduler.next_task();

                        if task.is_some() {
                            continue;
                        }

                        if scheduler.is_finish() {
                            break;
                        }

                        thread::yield_now();
                    }
                });
            }
        });

        if let Some(abort_reason) = self.abort_reason.take() {
            match abort_reason {
                AbortReason::FallbackToSequential => {
                    self.dropper.drop((mv_memory, scheduler, Vec::new()));
                    return execute_sequential(
                        db,
                        evm_env,
                        txs,
                        #[cfg(feature = "compiler")]
                        self.worker.clone(),
                    );
                }
                AbortReason::ExecutionError(err) => {
                    self.dropper.drop((mv_memory, scheduler, txs));
                    return Err(ParallelExecutorError::ExecutionError(err));
                }
            }
        }

        let mut fully_evaluated_results = Vec::with_capacity(block_size);
        let mut cumulative_gas_used: u64 = 0;
        for i in 0..block_size {
            let mut execution_result = index_mutex!(self.execution_results, i).take().unwrap();
            cumulative_gas_used =
                cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
            execution_result.receipt.cumulative_gas_used = cumulative_gas_used;
            fully_evaluated_results.push(execution_result);
        }

        // We fully evaluate (the balance and nonce of) the beneficiary account
        // and raw transfer recipients that may have been atomically updated.
        for address in mv_memory.consume_lazy_addresses() {
            let location_hash = hash_deterministic(MemoryLocation::Basic(address));
            if let Some(write_history) = mv_memory.data.get(&location_hash) {
                let mut balance = U256::ZERO;
                let mut nonce = 0;
                let mut code_hash = KECCAK_EMPTY;
                // Read from DB if the first multi-version entry is not an absolute value.
                if !matches!(
                    write_history.first_key_value(),
                    Some((_, MemoryEntry::Data(_, MemoryValue::Basic(_))))
                ) {
                    if let Ok(Some(account)) = db.basic_ref(address) {
                        balance = account.balance;
                        nonce = account.nonce;
                        code_hash = account.code_hash;
                    }
                }
                let code = match db.code_by_hash_ref(code_hash) {
                    Ok(code) => code,
                    Err(err) => return Err(ParallelExecutorError::StorageError(err.to_string())),
                };

                for (tx_idx, memory_entry) in write_history.iter() {
                    let tx = unsafe { txs.get_unchecked(*tx_idx) };
                    match memory_entry {
                        MemoryEntry::Data(_, MemoryValue::Basic(info)) => {
                            // We fall back to sequential execution when reading a self-destructed account,
                            // so an empty account here would be a bug
                            debug_assert!(!(info.balance.is_zero() && info.nonce == 0));
                            balance = info.balance;
                            nonce = info.nonce;
                        }
                        MemoryEntry::Data(_, MemoryValue::LazyRecipient(addition)) => {
                            balance = balance.saturating_add(*addition);
                        }
                        MemoryEntry::Data(_, MemoryValue::LazySender(subtraction)) => {
                            // We must re-do extra sender balance checks as we mock
                            // the max value in [Vm] during execution. Ideally we
                            // can turn off these redundant checks in revm.
                            // Ideally we would share these calculations with revm
                            // (using their utility functions).
                            let mut max_fee = U256::from(tx.gas_limit)
                                .saturating_mul(U256::from(tx.gas_price))
                                .saturating_add(tx.value);
                            {
                                max_fee = max_fee.saturating_add(
                                    U256::from(tx.total_blob_gas())
                                        .saturating_mul(U256::from(tx.max_fee_per_blob_gas)),
                                );
                            }
                            if balance < max_fee {
                                Err(ExecutionError::Transaction(
                                    InvalidTransaction::LackOfFundForMaxFee {
                                        balance: Box::new(balance),
                                        fee: Box::new(max_fee),
                                    },
                                ))?
                            }
                            balance = balance.saturating_sub(*subtraction);
                            nonce += 1;
                        }
                        // TODO: Better error handling
                        _ => unreachable!(),
                    }
                    // Assert that evaluated nonce is correct when address is caller.
                    if tx.caller == address {
                        let executed_nonce = if tx.nonce == 0 {
                            return Err(ParallelExecutorError::UnreachableError);
                        } else {
                            nonce - 1
                        };
                        if tx.nonce != executed_nonce {
                            // TODO: Consider falling back to sequential instead
                            return Err(ParallelExecutorError::NonceMismatch {
                                tx_idx: *tx_idx,
                                tx_nonce: tx.nonce,
                                executed_nonce,
                            });
                        }
                    }
                    // SAFETY
                    let tx_result = unsafe { fully_evaluated_results.get_unchecked_mut(*tx_idx) };
                    let contains_account = tx_result.state.contains_key(&address);

                    if evm_env.cfg_env.spec.is_enabled_in(SpecId::SPURIOUS_DRAGON)
                        && code_hash == KECCAK_EMPTY
                        && nonce == 0
                        && balance == U256::ZERO
                    {
                        // Do nothing for the empty account.
                    } else if contains_account {
                        // Only update the information except the code and code hash.
                        let account = tx_result.state.entry(address).or_default();
                        account.info.balance = balance;
                        account.info.nonce = nonce;
                    } else {
                        // Insert a new account.
                        tx_result.state.insert(
                            address,
                            Account {
                                info: AccountInfo {
                                    balance,
                                    nonce,
                                    code_hash,
                                    code: Some(code.clone()),
                                },
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }

        self.dropper.drop((mv_memory, scheduler, txs));

        Ok(fully_evaluated_results)
    }

    fn try_execute<DB: DatabaseRef + Send, T: TaskProvider>(
        &self,
        vm: &Vm<'_, DB>,
        scheduler: &Scheduler<T>,
        tx_version: TxVersion,
    ) -> Option<Task> {
        loop {
            return match vm.execute(&tx_version) {
                Err(VmExecutionError::Retry) => {
                    if self.abort_reason.get().is_none() {
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::FallbackToSequential) => {
                    self.abort_reason
                        .get_or_init(|| AbortReason::FallbackToSequential);
                    None
                }
                Err(VmExecutionError::Blocking(blocking_tx_idx)) => {
                    if !scheduler.add_dependency(tx_version.tx_idx, blocking_tx_idx)
                        && self.abort_reason.get().is_none()
                    {
                        // Retry the execution immediately if the blocking transaction was
                        // re-executed by the time we can add it as a dependency.
                        continue;
                    }
                    None
                }
                Err(VmExecutionError::ExecutionError(err)) => {
                    self.abort_reason
                        .get_or_init(|| AbortReason::ExecutionError(err));
                    None
                }
                Ok(VmExecutionResult {
                    execution_result,
                    flags,
                    affected_txs,
                }) => {
                    {
                        *index_mutex!(self.execution_results, tx_version.tx_idx) =
                            Some(execution_result);
                    }
                    scheduler.finish_execution(tx_version, flags, affected_txs)
                }
            };
        }
    }
}

#[inline]
fn try_validate<T: TaskProvider>(
    mv_memory: &MvMemory,
    scheduler: &Scheduler<T>,
    tx_idx: TxIdx,
) -> Option<Task> {
    let read_set_valid = mv_memory.validate_read_locations(tx_idx);
    scheduler.finish_validation(tx_idx, !read_set_valid)
}

/// Execute transactions sequentially.
/// Useful for falling back for (small) blocks with many dependencies.
pub fn execute_sequential<DB: DatabaseRef>(
    db: DB,
    evm_env: EvmEnv,
    txs: Vec<TxEnv>,
    #[cfg(feature = "compiler")] worker: Arc<ExtCompileWorker>,
) -> ParallelExecutorResult {
    let mut db = CacheDB::new(db);
    let mut evm = build_evm(&mut db, evm_env);
    let mut results = Vec::with_capacity(txs.len());
    let mut cumulative_gas_used: u64 = 0;
    for tx in txs {
        let tx_type = reth_primitives::TxType::try_from(tx.tx_type)
            .map_err(|_| ParallelExecutorError::UnreachableError)?;
        #[cfg(feature = "compiler")]
        let result_and_state = {
            use revm::handler::Handler;

            let mut t = metis_vm::CompilerHandler::new(worker.clone());
            evm.set_tx(tx);
            t.run(&mut evm)
                .map_err(|err| ExecutionError::Custom(err.to_string()))?
        };
        #[cfg(not(feature = "compiler"))]
        let result_and_state = {
            use revm::ExecuteEvm;

            evm.transact(tx)
                .map_err(|err| ExecutionError::Custom(err.to_string()))?
        };

        let mut execution_result = TxExecutionResult::from_raw(tx_type, result_and_state);

        cumulative_gas_used =
            cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
        execution_result.receipt.cumulative_gas_used = cumulative_gas_used;

        results.push(execution_result);
    }
    Ok(results)
}
