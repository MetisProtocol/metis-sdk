#[cfg(feature = "compiler")]
use std::sync::Arc;
use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{Mutex, OnceLock, mpsc},
    thread,
};

use alloy_primitives::{TxNonce, U256};
use alloy_rpc_types_eth::{Block, BlockTransactions};
use hashbrown::HashMap;
use metis_primitives::Transaction;
#[cfg(feature = "compiler")]
use metis_vm::ExtCompileWorker;
#[cfg(feature = "compiler")]
use revm::ExecuteEvm;
use revm::{DatabaseCommit, context::{BlockEnv, ContextTr, TxEnv, result::InvalidTransaction}, database::CacheDB, primitives::hardfork::SpecId, Database};
use revm::state::Bytecode;
use crate::{
    EvmAccount, MemoryEntry, MemoryLocation, MemoryValue, Task, TxIdx, TxVersion,
    chain::Chain,
    compat::get_block_env,
    hash_deterministic,
    mv_memory::MvMemory,
    scheduler::Scheduler,
    storage::StorageWrapper,
    vm::{ExecutionError, TxExecutionResult, Vm, VmExecutionError, VmExecutionResult, build_evm},
};
use crate::storage::Storage;

/// Errors when executing a block with the parallel executor.
// TODO: implement traits explicitly due to trait bounds on `C` instead of types of `Chain`
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParallelExecutorError<C: Chain> {
    /// Cannot derive the chain spec from the block header.
    #[error("Cannot derive the chain spec from the block header")]
    BlockSpecError(#[source] C::BlockSpecError),
    /// Transactions lack information for execution.
    #[error("Transactions lack information for execution")]
    MissingTransactionData,
    /// Invalid input transaction.
    #[error("Invalid input transaction")]
    InvalidTransaction(#[source] C::TransactionParsingError),
    /// Nonce too low or too high
    #[error("Nonce mismatch for tx #{tx_idx}. Expected {executed_nonce}, got {tx_nonce}")]
    NonceMismatch {
        /// Transaction index
        tx_idx: TxIdx,
        /// Nonce from tx (from the very input)
        tx_nonce: TxNonce,
        /// Nonce from state and execution
        executed_nonce: TxNonce,
    },
    /// Storage error.
    // TODO: More concrete types than just an arbitrary string.
    #[error("Storage error: {0}")]
    StorageError(String),
    /// EVM execution error.
    #[error("Execution error")]
    ExecutionError(
        #[source]
        #[from]
        ExecutionError,
    ),
    /// Impractical errors that should be unreachable.
    /// The library has bugs if this is yielded.
    #[error("Unreachable error")]
    UnreachableError,
}

/// Execution result of a block
pub type ParallelExecutorResult<C> = Result<Vec<TxExecutionResult>, ParallelExecutorError<C>>;

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

// TODO: Port more recyclable resources into here.
/// The main executor struct that executes blocks.
#[derive(Debug)]
#[cfg_attr(not(feature = "compiler"), derive(Default))]
pub struct ParallelExecutor {
    execution_results: Vec<Mutex<Option<TxExecutionResult>>>,
    abort_reason: OnceLock<AbortReason>,
    dropper: AsyncDropper<(MvMemory, Scheduler, Vec<TxEnv>)>,
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
    /// New a parrallel VM with the compiler feature.
    #[cfg(feature = "compiler")]
    pub fn compiler() -> Self {
        Self {
            worker: Arc::new(ExtCompileWorker::aot().expect("compile worker init failed")),
            ..Default::default()
        }
    }
}

impl ParallelExecutor {
    pub fn execute<S, C>(
        &mut self,
        chain: &C,
        storage: Mutex<S> ,
        // We assume the block is still needed afterwards like in most Reth cases
        // so take in a reference and only copy values when needed. We may want
        // to use a [`std::borrow::Cow`] to build [`BlockEnv`] and [`TxEnv`] without
        // (much) copying when ownership can be given. Another challenge with this is
        // the new Alloy [`Transaction`] interface that is mostly `&self`. We'd need
        // to do some dirty destruction to get the owned fields.
        block: &Block<C::Transaction>,
        concurrency_level: NonZeroUsize,
        force_sequential: bool,
    ) -> ParallelExecutorResult<C>
    where
        C: Chain + Send + Sync,
        S: Database + Storage + Debug + Send + Sync,
    {
        let spec_id = chain.spec();
        let block_env = get_block_env(&block.header, spec_id);
        let tx_envs = match &block.transactions {
            BlockTransactions::Full(txs) => txs
                .iter()
                .map(|tx| chain.get_tx_env(tx))
                .collect::<Result<Vec<TxEnv>, _>>()
                .map_err(ParallelExecutorError::InvalidTransaction)?,
            _ => return Err(ParallelExecutorError::MissingTransactionData),
        };
        // TODO: Continue to fine tune this condition.
        if force_sequential
            || tx_envs.len() < concurrency_level.get()
            || block.header.gas_used < 4_000_000
        {
            execute_revm_sequential(
                chain,
                storage,
                spec_id,
                block_env,
                tx_envs,
                #[cfg(feature = "compiler")]
                self.worker.clone(),
            )
        } else {
            self.execute_revm_parallel(
                chain,
                storage,
                spec_id,
                block_env,
                tx_envs,
                concurrency_level,
            )
        }
    }

    /// Execute an REVM block.
    // Ideally everyone would go through the [Alloy] interface. This one is currently
    // useful for testing, and for users that are heavily tied to Revm like Reth.
    pub fn execute_revm_parallel<S, C>(
        &mut self,
        chain: &C,
        storage: Mutex<S>,
        spec_id: SpecId,
        block_env: BlockEnv,
        txs: Vec<TxEnv>,
        concurrency_level: NonZeroUsize,
    ) -> ParallelExecutorResult<C>
    where
        C: Chain + Send + Sync,
        S: Database + Debug + Send + Sync + Storage,
    {
        if txs.is_empty() {
            return Ok(Vec::new());
        }

        let block_size = txs.len();
        let scheduler = Scheduler::new(block_size);

        let mv_memory = chain.build_mv_memory(&block_env, &txs);
        let vm = Vm::new(
            &storage,
            &mv_memory,
            chain,
            &block_env,
            &txs,
            spec_id,
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
                    while task.is_some() {
                        task = match task.unwrap() {
                            Task::Execution(tx_version) => {
                                self.try_execute(&vm, &scheduler, tx_version)
                            }
                            Task::Validation(tx_version) => {
                                try_validate(&mv_memory, &scheduler, &tx_version)
                            }
                        };

                        // TODO: Have different functions or an enum for the caller to choose
                        // the handling behaviour when a transaction's EVM execution fails.
                        // Parallel block builders would like to exclude such transaction,
                        // verifiers may want to exit early to save CPU cycles, while testers
                        // may want to collect all execution results. We are exiting early as
                        // the default behaviour for now.
                        if self.abort_reason.get().is_some() {
                            break;
                        }

                        if task.is_none() {
                            task = scheduler.next_task();
                        }
                    }
                });
            }
        });

        if let Some(abort_reason) = self.abort_reason.take() {
            match abort_reason {
                AbortReason::FallbackToSequential => {
                    self.dropper.drop((mv_memory, scheduler, Vec::new()));
                    return execute_revm_sequential(
                        chain,
                        storage,
                        spec_id,
                        block_env,
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
                // Read from storage if the first multi-version entry is not an absolute value.
                if !matches!(
                    write_history.first_key_value(),
                    Some((_, MemoryEntry::Data(_, MemoryValue::Basic(_))))
                ) {
                    if let Ok(Some(account)) = storage.lock().unwrap().basic(address) {
                        balance = account.balance;
                        nonce = account.nonce;
                    }
                }
                // Accounts that take implicit writes like the beneficiary account can be contract!
                let code_hash = match storage.lock().unwrap().code_hash(&address) {
                    Ok(code_hash) => code_hash,
                    Err(err) => return Err(ParallelExecutorError::StorageError(err.to_string())),
                };
                let code = if let Some(code_hash) = &code_hash {
                    match storage.lock().unwrap().code_by_hash(*code_hash) {
                        Ok(code) => code,
                        Err(err) => {
                            return Err(ParallelExecutorError::StorageError(err.to_string()));
                        }
                    }
                } else {
                    Bytecode::default()
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
                    // SAFETY: The multi-version data structure should not leak an index over block size.
                    let tx_result = unsafe { fully_evaluated_results.get_unchecked_mut(*tx_idx) };
                    let account = tx_result.state.entry(address).or_default();
                    // TODO: Deduplicate this logic with [TxExecutionResult::from_revm]
                    if chain.is_eip_161_enabled(spec_id)
                        && code_hash.is_none()
                        && nonce == 0
                        && balance == U256::ZERO
                    {
                        *account = None;
                    } else if let Some(account) = account {
                        // Explicit write: only overwrite the account info in case there are storage changes
                        // Code cannot change midblock here as we're falling back to sequential execution
                        // on reading a self-destructed contract.
                        account.balance = balance;
                        account.nonce = nonce;
                    } else {
                        // Implicit write: e.g. gas payments to the beneficiary account,
                        // which doesn't have explicit writes in [tx_result.state]
                        *account = Some(EvmAccount {
                            balance,
                            nonce,
                            code_hash,
                            code: Some(code.clone()),
                            storage: HashMap::default(),
                        });
                    }
                }
            }
        }

        self.dropper.drop((mv_memory, scheduler, txs));

        Ok(fully_evaluated_results)
    }

    fn try_execute<DB: Database + Storage, C: Chain>(
        &self,
        vm: &Vm<'_, DB, C>,
        scheduler: &Scheduler,
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
                    scheduler.abort();
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
                    scheduler.abort();
                    self.abort_reason
                        .get_or_init(|| AbortReason::ExecutionError(err));
                    None
                }
                Ok(VmExecutionResult {
                    execution_result,
                    flags,
                }) => {
                    *index_mutex!(self.execution_results, tx_version.tx_idx) =
                        Some(execution_result);
                    scheduler.finish_execution(tx_version, flags)
                }
            };
        }
    }
}

fn try_validate(
    mv_memory: &MvMemory,
    scheduler: &Scheduler,
    tx_version: &TxVersion,
) -> Option<Task> {
    let read_set_valid = mv_memory.validate_read_locations(tx_version.tx_idx);
    let aborted = !read_set_valid && scheduler.try_validation_abort(tx_version);
    if aborted {
        mv_memory.convert_writes_to_estimates(tx_version.tx_idx);
    }
    scheduler.finish_validation(tx_version, aborted)
}

/// Execute REVM transactions sequentially.
// Useful for falling back for (small) blocks with many dependencies.
// TODO: Use this for a long chain of sequential transactions even in parallel mode.
pub fn execute_revm_sequential<DB: Database + Storage + Debug, C: Chain>(
    chain: &C,
    storage: Mutex<DB>,
    spec_id: SpecId,
    block_env: BlockEnv,
    txs: Vec<TxEnv>,
    #[cfg(feature = "compiler")] worker: Arc<ExtCompileWorker>,
) -> ParallelExecutorResult<C> {
    let mut db = CacheDB::new(StorageWrapper(&storage));
    let mut evm = build_evm(&mut db, spec_id, block_env);
    let mut results = Vec::with_capacity(txs.len());
    let mut cumulative_gas_used: u64 = 0;
    for tx in txs {
        #[cfg(feature = "compiler")]
        let result_and_state = {
            use revm::handler::Handler;

            let mut t = metis_vm::CompilerHandler::new(worker.clone());
            evm.set_tx(tx);
            t.run(&mut evm)
                .map_err(|err| ExecutionError::Custom(err.to_string()))?
        };
        // TODO: complex error " method cannot be called due to unsatisfied trait bounds"
        #[cfg(not(feature = "compiler"))]
        let result_and_state = {
            use revm::ExecuteEvm;

            evm.transact(tx)
                .map_err(|err| ExecutionError::Custom(err.to_string()))?
        };
        evm.db().commit(result_and_state.state.clone());

        let mut execution_result = TxExecutionResult::from_revm(chain, spec_id, result_and_state);

        cumulative_gas_used =
            cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
        execution_result.receipt.cumulative_gas_used = cumulative_gas_used;

        results.push(execution_result);
    }
    Ok(results)
}
