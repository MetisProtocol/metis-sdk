use crate::hook_provider::MyEvmFactory;
use crate::state::StateStorageAdapter;
use alloy_consensus::{Header, Transaction};
use alloy_eips::eip7685::Requests;
use alloy_evm::block::{
    BlockExecutionError, BlockValidationError, CommitChanges,
    state_changes::post_block_balance_increments,
};
use alloy_evm::eth::dao_fork;
use alloy_evm::eth::dao_fork::DAO_HARDFORK_BENEFICIARY;
use alloy_evm::{Database, FromRecoveredTx, FromTxWithEncoded};
use alloy_hardforks::EthereumHardfork;
use alloy_rpc_types_engine::ExecutionData;
use metis_primitives::{CfgEnv, ExecutionResult, SpecId, TxEnv};
use reth::api::{FullNodeTypes, NodeTypes};
use reth::builder::BuilderContext;
use reth::builder::components::ExecutorBuilder;
use reth::{providers::BlockExecutionResult, revm::db::State};
use reth_chainspec::{ChainSpec, EthChainSpec, Hardforks};
use reth_ethereum_primitives::{Block, EthPrimitives, Receipt, TransactionSigned};
use reth_evm::TransactionEnv;
use reth_evm::block::{BlockExecutorFactory, BlockExecutorFor};
use reth_evm::block::{ExecutableTx, InternalBlockExecutionError};
use reth_evm::eth::spec::EthExecutorSpec;
use reth_evm::eth::{EthBlockExecutionCtx, EthBlockExecutor};
use reth_evm::precompiles::PrecompilesMap;
use reth_evm::{
    ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
};
use reth_evm::{EthEvmFactory, Evm, EvmEnv, EvmFactory, NextBlockEnvAttributes};
use reth_evm::{OnStateHook, execute::BlockExecutor};
pub use reth_evm_ethereum::EthEvmConfig;
use reth_evm_ethereum::{EthBlockAssembler, RethReceiptBuilder};
use reth_primitives_traits::{SealedBlock, SealedHeader};
use revm::Database as RevmDatabase;
use revm::{context::BlockEnv as RevmBlockEnv, context::result::ResultAndState};
use std::convert::Infallible;
use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Trait for executors that support batch parallel execution.
///
/// This trait allows Payload Builders to detect and use parallel execution
/// capabilities when available, falling back to serial execution otherwise.
pub trait BatchExecutable: BlockExecutor {
    /// Execute multiple transactions in a batch, potentially in parallel.
    ///
    /// # Arguments
    /// * `transactions` - Iterator of transactions that implement ExecutableTx
    ///
    /// # Returns
    /// Returns the block execution result including receipts, gas used, and requests.
    fn execute_transactions_batch<Tx>(
        &mut self,
        transactions: impl IntoIterator<Item = Tx>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError>
    where
        Tx: ExecutableTx<Self>;
}

/// Ethereum-related EVM configuration with the parallel executor.
#[derive(Debug, Clone)]
pub struct ParallelEthEvmConfig<C = ChainSpec, EvmFactory = EthEvmFactory> {
    pub config: EthEvmConfig<C, EvmFactory>,
}

impl<ChainSpec> ParallelEthEvmConfig<ChainSpec> {
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            config: EthEvmConfig::new(chain_spec),
        }
    }
}

impl<ChainSpec, EvmFactory> ParallelEthEvmConfig<ChainSpec, EvmFactory> {
    pub fn new_with_evm_factory(chain_spec: Arc<ChainSpec>, evm_factory: EvmFactory) -> Self {
        Self {
            config: EthEvmConfig::new_with_evm_factory(chain_spec, evm_factory),
        }
    }
}

impl<ChainSpec, EvmF> BlockExecutorFactory for ParallelEthEvmConfig<ChainSpec, EvmF>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + Clone + 'static,
    EvmF: EvmFactory<
            Tx = TxEnv,
            Spec = SpecId,
            BlockEnv = revm::context::BlockEnv,
            Precompiles = PrecompilesMap,
        > + Clone
        + Debug
        + Send
        + Sync
        + Unpin
        + 'static,
    EvmF::Tx:
        TransactionEnv + FromRecoveredTx<TransactionSigned> + FromTxWithEncoded<TransactionSigned>,
{
    type EvmFactory = EvmF;
    type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
    type Transaction = TransactionSigned;
    type Receipt = Receipt;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.config.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <Self::EvmFactory as EvmFactory>::Evm<&'a mut State<DB>, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> impl BlockExecutorFor<'a, Self, DB, I>
    where
        DB: Database + 'a,
        I: reth::revm::Inspector<<Self::EvmFactory as EvmFactory>::Context<&'a mut State<DB>>> + 'a,
    {
        ParallelBlockExecutor {
            spec: self.config.chain_spec().clone(),
            executor: EthBlockExecutor::new(
                evm,
                ctx,
                self.config.chain_spec().clone(),
                *self.config.executor_factory.receipt_builder(),
            ),
            context: crate::parallel_execution_context::ParallelExecutionContext::default(),
        }
    }
}

impl<ChainSpec, EvmF> ConfigureEvm for ParallelEthEvmConfig<ChainSpec, EvmF>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + Clone + 'static,
    EvmF: EvmFactory<
            Tx = TxEnv,
            Spec = SpecId,
            BlockEnv = revm::context::BlockEnv,
            Precompiles = PrecompilesMap,
        > + Clone
        + Debug
        + Send
        + Sync
        + Unpin
        + 'static,
    EvmF::Tx:
        TransactionEnv + FromRecoveredTx<TransactionSigned> + FromTxWithEncoded<TransactionSigned>,
{
    type Primitives = EthPrimitives;
    type Error = Infallible;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = Self;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        &self.config.block_assembler
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.config.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.config.next_evm_env(parent, attributes)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<EthBlockExecutionCtx<'a>, Self::Error> {
        self.config.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<EthBlockExecutionCtx<'_>, Self::Error> {
        self.config.context_for_next_block(parent, attributes)
    }
}

impl<EvmF> ConfigureEngineEvm<ExecutionData> for ParallelEthEvmConfig<ChainSpec, EvmF>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + Clone + 'static,
    EvmF: EvmFactory<
            Tx = TxEnv,
            Spec = SpecId,
            BlockEnv = revm::context::BlockEnv,
            Precompiles = PrecompilesMap,
        > + Clone
        + Debug
        + Send
        + Sync
        + Unpin
        + 'static,
    EvmF::Tx:
        TransactionEnv + FromRecoveredTx<TransactionSigned> + FromTxWithEncoded<TransactionSigned>,
{
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> Result<EvmEnvFor<Self>, Self::Error> {
        self.config.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.config.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        self.config.tx_iterator_for_payload(payload)
    }
}

/// Parallel block executor for Ethereum.
///
/// This executor provides both serial and parallel execution capabilities:
/// - Buffered batch parallel execution: execute_transaction() buffers txs, flush_pending() executes in parallel
/// - Immediate serial execution: For streaming validation scenarios
///
/// The executor ensures post_execution is called exactly once per block using
/// an internal context tracker.
pub struct ParallelBlockExecutor<'a, Evm, Spec> {
    /// Reference to the specification object.
    spec: Spec,
    /// Eth original executor (used for serial execution fallback).
    executor: EthBlockExecutor<'a, Evm, Spec, RethReceiptBuilder>,
    /// Execution context to track state and prevent double post_execution calls
    /// Contains execution_cache for avoiding double execution
    context: crate::parallel_execution_context::ParallelExecutionContext,
}

impl<'db, DB, E, Spec> BlockExecutor for ParallelBlockExecutor<'_, E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv, BlockEnv = revm::context::BlockEnv>,
    Spec: EthExecutorSpec + EthChainSpec + Hardforks + Clone,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.executor.apply_pre_execution_changes()
    }

    fn execute_transaction_with_result_closure(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as reth_evm::Evm>::HaltReason>),
    ) -> Result<u64, BlockExecutionError> {
        self.executor.execute_transaction_with_result_closure(tx, f)
        // let tx_hash = *tx.tx().hash();
        //
        // // Check global cache first (set by payload builder)
        // if let Some(cached) = crate::parallel_execution_context::get_global_cached_result(&tx_hash)
        // {
        //     // Call the callback with a dummy execution result (since we already have the receipt)
        //     let dummy_result = ExecutionResult::Success {
        //         reason: SuccessReason::Stop,
        //         gas_used: cached.gas_used,
        //         gas_refunded: 0,
        //         logs: cached.receipt.logs.clone(),
        //         output: Output::Call(metis_primitives::Bytes::new()),
        //     };
        //     f(&dummy_result);
        //
        //     return Ok(cached.gas_used);
        // }
        //
        // // Check local cache (for backwards compatibility)
        // if let Some(cached) = self.context.get_cached_result(&tx_hash) {
        //     let dummy_result = ExecutionResult::Success {
        //         reason: SuccessReason::Stop,
        //         gas_used: cached.gas_used,
        //         gas_refunded: 0,
        //         logs: cached.receipt.logs.clone(),
        //         output: Output::Call(metis_primitives::Bytes::new()),
        //     };
        //     f(&dummy_result);
        //
        //     return Ok(cached.gas_used);
        // }
        //
        // // CRITICAL FIX: Execute transactions IMMEDIATELY in Payload Validator mode.
        // //
        // // Problem: In Payload Validator mode, Reth uses Streaming API:
        // // 1. Calls execute_transaction() multiple times to buffer transactions
        // // 2. Starts parallel state root calculation IMMEDIATELY (uses current DB snapshot)
        // // 3. Calls finish() to execute buffered transactions
        // // 4. State root calculation completes using OLD snapshot from step 2
        // //
        // // Solution: Execute each transaction immediately when called.
        // // Trade-off: Loses parallelism in Payload Validator mode, but ensures correctness.
        // // Note: Payload Builder mode (with execute_transactions()) still uses parallel execution.
        //
        // // Execute immediately using the inner executor (serial execution)
        // let result = self
        //     .executor
        //     .execute_transaction_with_result_closure(tx, f)?;
        //
        // Ok(result)
    }

    fn execute_transaction_with_commit_condition(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as Evm>::HaltReason>) -> CommitChanges,
    ) -> Result<Option<u64>, BlockExecutionError> {
        self.executor
            .execute_transaction_with_commit_condition(tx, f)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        // Apply post_execution (block rewards, withdrawals, etc.)
        // This is called once per block after all transactions execute
        // if !self.context.post_execution_called {
        //     self.post_execution()?;
        //     self.context.mark_post_execution_called();
        // }

        // Delegate to inner executor for final state root calculation
        self.executor.finish()
    }

    fn set_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.executor.set_state_hook(hook);
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.executor.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.executor.evm()
    }

    /// Executes all transactions in a block, applying pre and post execution changes.
    /// This method triggers PARALLEL execution using metis-pe.
    fn execute_block(
        mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Self::Receipt>, BlockExecutionError>
    where
        Self: Sized,
    {
        self.apply_pre_execution_changes()?;
        let result = self.execute(transactions)?;
        Ok(result)
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<ResultAndState<<Self::Evm as Evm>::HaltReason>, BlockExecutionError> {
        self.executor.execute_transaction_without_commit(tx)
    }

    fn execute_transaction(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        let tx_hash = *tx.tx().hash();

        // Check if we have cached result from parallel execution
        if let Some(cached) = self.context.get_cached_result(&tx_hash) {
            // Return cached gas_used
            // The transaction is NOT re-executed, we just use the cached result
            // This allows builder to add the transaction to the block structure
            return Ok(cached.gas_used);
        }

        // No cache - execute normally (serial execution)
        self.execute_transaction_with_result_closure(tx, |_| ())
    }

    fn commit_transaction(
        &mut self,
        output: ResultAndState<<Self::Evm as Evm>::HaltReason>,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        self.executor.commit_transaction(output, tx)
    }
}

impl<'db, DB, E, Spec> ParallelBlockExecutor<'_, E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv, BlockEnv = revm::context::BlockEnv>,
    Spec: EthExecutorSpec + EthChainSpec + Hardforks + Clone,
{
    pub fn execute(
        &mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {
        tracing::debug!(target: "metis::parallel",
            "ParallelBlockExecutor::execute() - parallel execution entry point"
        );

        let block_env: &RevmBlockEnv = self.evm().block();
        let state_clear_flag = self
            .spec
            .is_spurious_dragon_active_at_block(block_env.number.try_into().unwrap_or(u64::MAX));

        // Build CfgEnv for metis-pe based on the current block.
        //
        // NOTE: We must ensure fork rules match reth's canonical env, otherwise state roots can
        // diverge. (The `block_env` itself is cloned from the canonical EVM instance.)
        let block_number = block_env.number.try_into().unwrap_or(u64::MAX);
        let block_timestamp = block_env.timestamp.to::<u64>();

        let spec_id = if self.spec.is_cancun_active_at_timestamp(block_timestamp) {
            SpecId::CANCUN
        } else if self.spec.is_shanghai_active_at_timestamp(block_timestamp) {
            SpecId::SHANGHAI
        } else if self.spec.is_london_active_at_block(block_number) {
            SpecId::LONDON
        } else if self.spec.is_berlin_active_at_block(block_number) {
            SpecId::BERLIN
        } else {
            SpecId::ISTANBUL
        };

        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = self.spec.chain_id();
        cfg_env.spec = spec_id;

        let block_env = self.evm().block().clone();

        let evm_env = EvmEnv::new(cfg_env, block_env);
        let db = self.evm_mut().db_mut();
        db.set_state_clear_flag(state_clear_flag);

        // Collect transactions and calculate blob gas used
        let transactions: Vec<_> = transactions.into_iter().collect();
        let total_blob_gas_used: u64 = transactions
            .iter()
            .filter_map(|tx| tx.tx().blob_gas_used())
            .sum();

        let num_threads = num_cpus::get();

        let pe_start_time = std::time::Instant::now();
        // IMPORTANT: align metis-pe ResultAndState<HR> with the executor's concrete HaltReason type
        let mut parallel_executor =
            metis_pe::ParallelExecutor::<<E as alloy_evm::Evm>::HaltReason>::default();

        // Build TxEnv list without consuming the original txs so we can commit in-order later
        let transactions_vec: Vec<_> = transactions.into_iter().collect();
        let tx_envs: Vec<TxEnv> = transactions_vec.iter().map(|tx| tx.to_tx_env()).collect();

        let results = parallel_executor.execute(
            StateStorageAdapter::new(db),
            evm_env,
            tx_envs,
            NonZeroUsize::new(num_threads).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );

        let pe_duration = pe_start_time.elapsed();
        tracing::debug!(target: "metis::parallel",
            "Parallel execution completed: duration={:?}",
            pe_duration
        );

        let results_vec = results.map_err(|err| {
            tracing::error!(target: "metis::parallel", "Parallel execution failed: {:?}", err);
            BlockExecutionError::Internal(InternalBlockExecutionError::Other(Box::new(err)))
        })?;

        // Commit pre-executed ResultAndState using the executor's native commit semantics.
        //
        // This is REQUIRED for correct state roots because it goes through revm's journal / bundle
        // and reth's commit logic (account clearing, selfdestruct, created contracts, etc.).

        let mut receipts: Vec<Receipt> = Vec::with_capacity(results_vec.len());

        for (tx, result) in transactions_vec.into_iter().zip(results_vec.into_iter()) {
            receipts.push(result.receipt.clone());

            // Apply state exactly as reth would after executing the tx
            // IMPORTANT:
            // revm::db::State will panic if we commit accounts that are not present in its internal
            // cache ("All accounts should be present inside cache"). Prewarm the cache for every
            // touched address in this tx's state before committing.
            {
                let mut addrs: Vec<_> = result.result_and_state.state.keys().copied().collect();
                addrs.sort_unstable();
                let db = self.evm_mut().db_mut();
                for addr in addrs {
                    db.basic(addr).map_err(|err| {
                        BlockExecutionError::Internal(InternalBlockExecutionError::Other(Box::new(
                            err,
                        )))
                    })?;
                }
            }
            self.commit_transaction(result.result_and_state, tx)?;
        }

        // The total gas used is the cumulative_gas_used of the last receipt
        // (which already includes all previous transactions' gas)
        let total_gas_used = receipts.last().map(|r| r.cumulative_gas_used).unwrap_or(0);

        // FIX: Call post_execution() here to ensure block rewards are applied.
        //
        // Problem: In some paths (like execute_block), finish() may not be called,
        // leading to missing block rewards and state root mismatches.
        //
        // Solution: Call post_execution() here with a flag to prevent double application.
        // The flag ensures it's only called once per block execution.
        if !self.context.post_execution_called {
            self.post_execution()?;
            self.context.mark_post_execution_called();
        }

        // Parse EIP-7685 requests (withdrawal, deposit, consolidation) if Prague is active
        let block_env: &revm::context::BlockEnv = self.executor.evm.block();
        let requests = if self
            .spec
            .is_prague_active_at_timestamp(block_env.timestamp.to::<u64>())
        {
            // Use the inner executor's system_caller to parse requests from system contracts
            self.executor
                .system_caller
                .apply_post_execution_changes(&mut self.executor.evm)?
        } else {
            Requests::default()
        };

        let result = BlockExecutionResult {
            receipts,
            requests,
            gas_used: total_gas_used,
            blob_gas_used: total_blob_gas_used,
        };

        Ok(result)
    }

    fn post_execution(&mut self) -> Result<(), BlockExecutionError> {
        let mut balance_increments = post_block_balance_increments(
            &self.spec,
            self.evm().block(),
            self.executor.ctx.ommers,
            self.executor.ctx.withdrawals.as_deref(),
        );
        // Irregular state change at Ethereum DAO hardfork
        let block_env: &RevmBlockEnv = self.evm().block();
        if self
            .spec
            .ethereum_fork_activation(EthereumHardfork::Dao)
            .transitions_at_block(block_env.number.try_into().unwrap_or(u64::MAX))
        {
            // drain balances from hardcoded addresses.
            let drained_balance: u128 = self
                .evm_mut()
                .db_mut()
                .drain_balances(dao_fork::DAO_HARDFORK_ACCOUNTS)
                .map_err(|_| BlockValidationError::IncrementBalanceFailed)?
                .into_iter()
                .sum();

            // return balance to DAO beneficiary.
            *balance_increments
                .entry(DAO_HARDFORK_BENEFICIARY)
                .or_default() += drained_balance;
        }
        // CRITICAL: Sort balance_increments by address to ensure deterministic commit order
        // HashMap iteration order is non-deterministic, which can cause different state roots
        // across nodes even with the same execution results.
        // We need to commit in a deterministic order (sorted by address)
        let mut sorted_increments: Vec<_> = balance_increments.into_iter().collect();
        sorted_increments.sort_by_key(|(address, _)| *address);

        // CRITICAL: Use BTreeMap instead of HashMap to maintain sorted order.
        // BTreeMap preserves the sorted order when iterating, ensuring deterministic
        // state root calculation across all nodes.
        let sorted_balance_increments: std::collections::BTreeMap<_, _> =
            sorted_increments.into_iter().collect();

        // increment balances
        self.evm_mut()
            .db_mut()
            .increment_balances(sorted_balance_increments)
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;

        Ok(())
    }
}

// ====================================================================
// BatchExecutable Implementation for ParallelBlockExecutor
// ====================================================================

impl<'db, DB, E, Spec> BatchExecutable for ParallelBlockExecutor<'_, E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv, BlockEnv = revm::context::BlockEnv>,
    Spec: EthExecutorSpec + EthChainSpec + Hardforks + Clone,
{
    /// Execute multiple transactions in a batch using parallel execution.
    ///
    /// This method is called by the custom Payload Builder to enable
    /// true parallel transaction execution using Block-STM.
    fn execute_transactions_batch<Tx>(
        &mut self,
        transactions: impl IntoIterator<Item = Tx>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError>
    where
        Tx: ExecutableTx<Self>,
    {
        // Convert to a vec to count transactions
        let tx_vec: Vec<Tx> = transactions.into_iter().collect();

        // Call the existing execute method which handles parallel execution
        self.execute(tx_vec)
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = ParallelEthEvmConfig<ChainSpec, MyEvmFactory>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(ParallelEthEvmConfig::new_with_evm_factory(
            ctx.chain_spec(),
            MyEvmFactory::default(),
        ))
    }
}
