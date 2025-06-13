use crate::{
    Entry, ExecutionError, Location, LocationValue, Task, TxVersion,
    dropper::AsyncDropper,
    mv_memory::{MvMemory, build_op_mv_memory},
    op_vm::{OpVm, build_op_evm},
    result::{
        AbortReason, ParallelExecutorError, ParallelExecutorResult, TxExecutionResult,
        VmExecutionError, VmExecutionResult, evm_err_to_exec_error,
    },
    scheduler::{NormalProvider, Scheduler, TaskProvider},
};
use alloy_evm::EvmEnv;
#[cfg(feature = "compiler")]
use metis_primitives::{
    Account, AccountInfo, AccountStatus, CacheDB, ContextTr, DatabaseCommit, DatabaseRef,
    ExecuteEvm, InvalidTransaction, KECCAK_EMPTY, SpecId, Transaction, TxEnv, U256,
    hash_deterministic,
};
#[cfg(feature = "compiler")]
use metis_vm::ExtCompileWorker;
use op_revm::OpTransaction;
#[cfg(feature = "compiler")]
use std::sync::Arc;
use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{Mutex, OnceLock},
    thread,
};

/// The main executor struct that executes blocks with Block-STM algorithm.
#[derive(Debug)]
#[cfg_attr(not(feature = "compiler"), derive(Default))]
pub struct OpParallelExecutor {
    execution_results: Vec<Mutex<Option<TxExecutionResult>>>,
    abort_reason: OnceLock<AbortReason>,
    #[cfg(feature = "async-dropper")]
    dropper: AsyncDropper<(MvMemory, Scheduler<NormalProvider>, Vec<TxEnv>)>,
    /// The compile work shared with different vm instance.
    #[cfg(feature = "compiler")]
    pub worker: Arc<ExtCompileWorker>,
}

#[cfg(feature = "compiler")]
impl Default for OpParallelExecutor {
    fn default() -> Self {
        Self {
            execution_results: Default::default(),
            abort_reason: Default::default(),
            #[cfg(feature = "async-dropper")]
            dropper: Default::default(),
            worker: Arc::new(ExtCompileWorker::disable()),
        }
    }
}

impl OpParallelExecutor {
    /// New a parallel VM with the compiler feature. The default compiler is an AOT-based one.
    #[cfg(feature = "compiler")]
    pub fn compiler() -> Self {
        Self {
            worker: Arc::new(ExtCompileWorker::aot().expect("compile worker init failed")),
            ..Default::default()
        }
    }
}

impl OpParallelExecutor {
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

        let mv_memory = build_op_mv_memory(&evm_env.block_env, &txs);
        let vm = OpVm::new(
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

        thread::scope(|scope| {
            for _ in 0..concurrency_level.into() {
                scope.spawn(|| {
                    let mut next_task = scheduler.next_task();
                    while let Some(task) = next_task {
                        next_task = match task {
                            Task::Execution(tx_version) => {
                                self.try_execute(&vm, &scheduler, tx_version)
                            }
                            Task::Validation(tx_version) => {
                                try_validate(&mv_memory, &scheduler, &tx_version)
                            }
                        };
                        if self.abort_reason.get().is_some() {
                            break;
                        }
                        if next_task.is_none() {
                            next_task = scheduler.next_task();
                        }
                    }
                });
            }
        });

        if let Some(abort_reason) = self.abort_reason.take() {
            match abort_reason {
                AbortReason::FallbackToSequential => {
                    #[cfg(feature = "async-dropper")]
                    self.dropper.drop((mv_memory, scheduler, Vec::new()));
                    return op_execute_sequential(
                        db,
                        evm_env,
                        txs,
                        #[cfg(feature = "compiler")]
                        self.worker.clone(),
                    );
                }
                AbortReason::ExecutionError(err) => {
                    #[cfg(feature = "async-dropper")]
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
            let location_hash = hash_deterministic(Location::Basic(address));
            if let Some(write_history) = mv_memory.data.get(&location_hash) {
                let mut balance = U256::ZERO;
                let mut nonce = 0;
                let mut code_hash = KECCAK_EMPTY;
                // Read from DB if the first multi-version entry is not an absolute value.
                if !matches!(
                    write_history.first_key_value(),
                    Some((_, Entry::Data(_, LocationValue::Basic(_))))
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

                for (tx_idx, entry) in write_history.iter() {
                    let tx = unsafe { txs.get_unchecked(*tx_idx) };
                    match entry {
                        Entry::Data(_, LocationValue::Basic(info)) => {
                            // We fall back to sequential execution when reading a self-destructed account,
                            // so an empty account here would be a bug
                            debug_assert!(!(info.balance.is_zero() && info.nonce == 0));
                            balance = info.balance;
                            nonce = info.nonce;
                        }
                        Entry::Data(_, LocationValue::LazyRecipient(addition)) => {
                            balance = balance.saturating_add(*addition);
                        }
                        Entry::Data(_, LocationValue::LazySender(subtraction)) => {
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
                        _ => unreachable!(),
                    }
                    // Assert that evaluated nonce is correct when address is caller.
                    if tx.caller == address {
                        let executed_nonce = if nonce == 0 {
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
                        // Insert a new account with the touched status.
                        // Only when account is marked as touched we will save it to database.
                        tx_result.state.insert(
                            address,
                            Account {
                                info: AccountInfo {
                                    balance,
                                    nonce,
                                    code_hash,
                                    code: Some(code.clone()),
                                },
                                status: AccountStatus::Touched,
                                ..Default::default()
                            },
                        );
                    }
                }
            }
        }

        #[cfg(feature = "async-dropper")]
        self.dropper.drop((mv_memory, scheduler, txs));

        Ok(fully_evaluated_results)
    }

    fn try_execute<DB: DatabaseRef + Send, T: TaskProvider>(
        &self,
        vm: &OpVm<'_, DB>,
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
                    {
                        *index_mutex!(self.execution_results, tx_version.tx_idx) =
                            Some(execution_result);
                    }
                    scheduler.finish_execution(tx_version, flags)
                }
            };
        }
    }
}

#[inline]
fn try_validate<T: TaskProvider>(
    mv_memory: &MvMemory,
    scheduler: &Scheduler<T>,
    tx_version: &TxVersion,
) -> Option<Task> {
    let read_set_valid = mv_memory.validate_read_locations(tx_version.tx_idx);
    let aborted = !read_set_valid && scheduler.try_validation_abort(tx_version);
    if aborted {
        mv_memory.convert_writes_to_estimates(tx_version.tx_idx);
    }
    scheduler.finish_validation(tx_version, aborted)
}

/// Execute transactions sequentially.
/// Useful for falling back for (small) blocks with many dependencies.
pub fn op_execute_sequential<DB: DatabaseRef>(
    db: DB,
    evm_env: EvmEnv,
    txs: Vec<TxEnv>,
    #[cfg(feature = "compiler")] worker: Arc<ExtCompileWorker>,
) -> ParallelExecutorResult {
    let mut db = CacheDB::new(db);
    let mut evm = build_op_evm(&mut db, Default::default());
    let mut results = Vec::with_capacity(txs.len());
    let mut cumulative_gas_used: u64 = 0;
    for tx in txs {
        let tx_type = reth_primitives::TxType::try_from(tx.tx_type)
            .map_err(|_| ParallelExecutorError::UnreachableError)?;
        #[cfg(feature = "compiler")]
        let result_and_state = {
            use revm::handler::Handler;

            let mut t = metis_vm::CompilerHandler::new(worker.clone());
            evm.set_tx(OpTransaction::new(tx));
            t.run(&mut evm).map_err(evm_err_to_exec_error::<DB>)?
        };
        #[cfg(not(feature = "compiler"))]
        let result_and_state = {
            use revm::ExecuteEvm;

            evm.transact(tx).map_err(evm_err_to_exec_error::<DB>)?
        };

        evm.db().commit(result_and_state.state.clone());

        let mut execution_result = TxExecutionResult::from_raw(tx_type, result_and_state);

        cumulative_gas_used =
            cumulative_gas_used.saturating_add(execution_result.receipt.cumulative_gas_used);
        execution_result.receipt.cumulative_gas_used = cumulative_gas_used;

        results.push(execution_result);
    }
    Ok(results)
}
