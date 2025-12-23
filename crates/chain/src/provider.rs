use crate::hook_provider::MyEvmFactory;
use crate::state::StateStorageAdapter;
use alloy_consensus::{Header, Transaction};
use alloy_eips::eip7685::Requests;
use alloy_evm::block::{
    BlockExecutionError, BlockValidationError, CommitChanges, SystemCaller,
    state_changes::post_block_balance_increments,
};
use alloy_evm::eth::dao_fork::DAO_HARDFORK_BENEFICIARY;
use alloy_evm::eth::{dao_fork, eip6110};
use alloy_evm::{Database, FromRecoveredTx, FromTxWithEncoded};
use alloy_hardforks::EthereumHardfork;
use alloy_rpc_types_engine::ExecutionData;
use metis_primitives::{CfgEnv, ExecutionResult, Output, SpecId, SuccessReason, TxEnv};
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
        tracing::debug!("Creating ParallelBlockExecutor");

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

    /// 执行交易并立即提交状态变更
    ///
    /// **调用场景：**
    /// 1. **Block Builder 流式执行**：通过 `BasicBlockBuilder.execute_transaction()` 调用
    ///    - 用于构建新块（payload builder、dev mode mining）
    ///    - 用于验证 payload（payload validator）
    /// 2. **RPC 调用**：`eth_call`、`eth_estimateGas`、`eth_simulateV1` 等
    ///    - 通过 `simulate::execute_transactions()` 调用
    ///
    /// **特点：**
    /// - 执行后立即提交状态变更到数据库
    /// - 返回 gas_used
    /// - 会调用回调函数 `f` 处理执行结果
    fn execute_transaction_with_result_closure(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as reth_evm::Evm>::HaltReason>),
    ) -> Result<u64, BlockExecutionError> {
        static CALL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let tx_hash = *tx.tx().hash();

        tracing::info!(target: "metis::parallel",
            "🔍 execute_transaction_with_result_closure() CALLED [call #{}] for tx 0x{:x}",
            count, tx_hash
        );

        // Check global cache first (set by payload builder)
        if let Some(cached) = crate::parallel_execution_context::get_global_cached_result(&tx_hash)
        {
            tracing::info!(target: "metis::parallel", call_count=?count, tx_hash=?format!("{:?}", tx_hash),
                "✅ Using CACHED parallel execution result from GLOBAL cache (skipping re-execution)");

            // Call the callback with a dummy execution result (since we already have the receipt)
            let dummy_result = ExecutionResult::Success {
                reason: SuccessReason::Stop,
                gas_used: cached.gas_used,
                gas_refunded: 0,
                logs: cached.receipt.logs.clone(),
                output: Output::Call(metis_primitives::Bytes::new()),
            };
            f(&dummy_result);

            return Ok(cached.gas_used);
        }

        // Check local cache (for backwards compatibility)
        if let Some(cached) = self.context.get_cached_result(&tx_hash) {
            tracing::info!(target: "metis::parallel", call_count=?count, tx_hash=?format!("{:?}", tx_hash),
                "✅ Using CACHED parallel execution result from LOCAL cache (skipping re-execution)");

            let dummy_result = ExecutionResult::Success {
                reason: SuccessReason::Stop,
                gas_used: cached.gas_used,
                gas_refunded: 0,
                logs: cached.receipt.logs.clone(),
                output: Output::Call(metis_primitives::Bytes::new()),
            };
            f(&dummy_result);

            return Ok(cached.gas_used);
        }

        tracing::debug!(target: "metis::cli", call_count=?count, tx_hash=?format!("{:?}", tx_hash),
            "execute_transaction_with_result_closure() called - IMMEDIATE SERIAL EXECUTION");

        // CRITICAL FIX: Execute transactions IMMEDIATELY in Payload Validator mode.
        //
        // Problem: In Payload Validator mode, Reth uses Streaming API:
        // 1. Calls execute_transaction() multiple times to buffer transactions
        // 2. Starts parallel state root calculation IMMEDIATELY (uses current DB snapshot)
        // 3. Calls finish() to execute buffered transactions
        // 4. State root calculation completes using OLD snapshot from step 2
        //
        // Solution: Execute each transaction immediately when called.
        // Trade-off: Loses parallelism in Payload Validator mode, but ensures correctness.
        // Note: Payload Builder mode (with execute_transactions()) still uses parallel execution.

        // Execute immediately using the inner executor (serial execution)
        let result = self
            .executor
            .execute_transaction_with_result_closure(tx, f)?;

        Ok(result)
    }

    fn execute_transaction_with_commit_condition(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as Evm>::HaltReason>) -> CommitChanges,
    ) -> Result<Option<u64>, BlockExecutionError> {
        // Same immediate execution strategy as execute_transaction_with_result_closure()
        // Execute immediately using the inner executor (serial execution)
        self.executor
            .execute_transaction_with_commit_condition(tx, f)
    }

    fn finish(
        mut self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        tracing::info!(target: "metis::parallel", "ParallelBlockExecutor::finish() called");

        // Apply post_execution (block rewards, withdrawals, etc.)
        // This is called once per block after all transactions execute
        if !self.context.post_execution_called {
            tracing::info!(target: "metis::parallel", "Calling post_execution from finish()");
            self.post_execution()?;
            self.context.mark_post_execution_called();
        } else {
            tracing::debug!(target: "metis::parallel", "Skipping post_execution - already called");
        }

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
        tracing::info!(target: "metis::cli", "ParallelBlockExecutor: Executing block with transaction processing");
        self.apply_pre_execution_changes()?;
        let result = self.execute(transactions)?;

        // CRITICAL: For Payload Validator path, we need to ensure post_execution() is called
        // even if execute_transactions() didn't call execute() (e.g., for empty blocks).
        // However, execute_transactions() calls execute() which already calls post_execution(),
        // so we don't need to call it again here.
        //
        // But wait: in Payload Validator path, execute_transactions() is NOT called!
        // Instead, execute_transaction() is called multiple times, then finish() is called.
        // So for Payload Validator path, post_execution() should be called in finish().
        //
        // Actually, for Payload Validator path, execute_block() is NOT called!
        // Instead, execute_metered() calls execute_transaction() multiple times, then finish().
        // So we need to call post_execution() in finish() for Payload Validator path.

        Ok(result)
    }

    /// 执行交易但不提交状态变更
    ///
    /// **调用场景：**
    /// 1. **调试和追踪**：`debug_traceTransaction`、`trace_block` 等 RPC 方法
    ///    - 需要检查执行结果但不修改状态
    /// 2. **条件执行**：需要先检查执行结果再决定是否提交的场景
    ///    - 通常配合 `commit_transaction()` 使用
    ///
    /// **特点：**
    /// - 执行后不提交状态变更
    /// - 返回 `ResultAndState`，包含执行结果和状态变更
    /// - 需要后续调用 `commit_transaction()` 来提交状态
    ///
    /// **使用模式：**
    /// ```rust
    /// let result = executor.execute_transaction_without_commit(tx)?;
    /// // 检查 result.result 决定是否提交
    /// if should_commit {
    ///     executor.commit_transaction(result, tx)?;
    /// }
    /// ```
    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<ResultAndState<<Self::Evm as Evm>::HaltReason>, BlockExecutionError> {
        tracing::debug!(target: "metis::cli", 
            "execute_transaction_without_commit() called - execution without commit");
        self.executor.execute_transaction_without_commit(tx)
    }

    fn execute_transaction(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        let tx_hash = *tx.tx().hash();

        // Check if we have cached result from parallel execution
        if let Some(cached) = self.context.get_cached_result(&tx_hash) {
            tracing::info!(target: "metis::parallel",
                "✨ Using CACHED result for tx_hash=0x{:x}, gas_used={} (from parallel execution)",
                tx_hash, cached.gas_used
            );
            tracing::debug!(target: "metis::parallel",
                "   Mode: CACHED (from parallel execution)"
            );

            // Return cached gas_used
            // The transaction is NOT re-executed, we just use the cached result
            // This allows builder to add the transaction to the block structure
            return Ok(cached.gas_used);
        }

        // No cache - execute normally (serial execution)
        tracing::info!(target: "metis::parallel",
            "🔷 SERIAL execution for tx_hash=0x{:x} (no cache available)",
            tx_hash
        );
        tracing::debug!(target: "metis::parallel",
            "   Mode: IMMEDIATE SERIAL"
        );

        let result = self.execute_transaction_with_result_closure(tx, |_| ());

        if let Ok(gas_used) = result {
            tracing::debug!(target: "metis::parallel",
                "   ✅ Transaction executed: tx_hash=0x{:x}, gas_used={}",
                tx_hash,
                gas_used
            );
        }

        result
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
    /// Merge and commit parallel execution results using StateAccumulator
    fn merge_and_commit_parallel_results(
        &mut self,
        results: &[metis_pe::TxExecutionResult],
    ) -> Result<(), BlockExecutionError> {
        let mut accumulator = crate::state_accumulator::StateAccumulator::new();
        accumulator.accumulate(results);
        accumulator.commit_to(self.evm_mut().db_mut())
    }

    /// Cache parallel execution results for later use by execute_transaction()
    /// This is used in payload builder to avoid re-executing transactions
    pub fn cache_parallel_results(
        &mut self,
        tx_hashes: &[alloy_primitives::TxHash],
        results: &[metis_pe::TxExecutionResult],
    ) {
        tracing::info!(target: "metis::parallel",
            "💾 Caching {} parallel execution results",
            results.len()
        );

        for (tx_hash, result) in tx_hashes.iter().zip(results.iter()) {
            self.context.cache_result(
                *tx_hash,
                result.receipt.cumulative_gas_used,
                result.receipt.clone(),
            );
            tracing::debug!(target: "metis::parallel",
                "   Cached: tx_hash=0x{:x}, gas_used={}",
                tx_hash,
                result.receipt.cumulative_gas_used
            );
        }
    }

    pub fn execute(
        &mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {
        tracing::warn!(target: "metis::parallel",
            "🚀🚀🚀 PARALLEL EXECUTOR ACTIVE - EXECUTING TRANSACTIONS IN PARALLEL 🚀🚀🚀"
        );
        tracing::info!(target: "metis::parallel",
            "🔥 ParallelBlockExecutor::execute() - Entry point for REAL parallel execution"
        );
        tracing::debug!(target: "metis::parallel",
            "   Using Block-STM algorithm for optimistic parallel execution"
        );

        let block_env: &RevmBlockEnv = self.evm().block();
        let state_clear_flag = self
            .spec
            .is_spurious_dragon_active_at_block(block_env.number.try_into().unwrap_or(u64::MAX));

        // Build proper CfgEnv from spec and context
        let block_number = block_env.number.try_into().unwrap_or(u64::MAX);
        let block_timestamp = block_env.timestamp.to::<u64>();

        // Get the proper spec ID for the current block using EthereumHardforks trait
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

        // Clone and configure BlockEnv with proper blob gas settings
        let mut block_env = self.evm().block().clone();

        // For Cancun and later, we need to set blob gas fields
        // Use 0 for dev mode / fresh blocks (no parent blob gas usage)
        if spec_id >= SpecId::CANCUN {
            use revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN;
            block_env.set_blob_excess_gas_and_price(0, BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN);
        }

        // Save chain_id before moving cfg_env
        let chain_id = cfg_env.chain_id;
        let evm_env = EvmEnv::new(cfg_env, block_env);
        let db = self.evm_mut().db_mut();
        db.set_state_clear_flag(state_clear_flag);

        // Collect transactions and calculate blob gas used
        let transactions: Vec<_> = transactions.into_iter().collect();
        let tx_count = transactions.len();
        let total_blob_gas_used: u64 = transactions
            .iter()
            .filter_map(|tx| tx.tx().blob_gas_used())
            .sum();

        let num_threads = num_cpus::get();
        tracing::info!(target: "metis::parallel",
            "⚡ Calling metis_pe::ParallelExecutor: tx_count={}, threads={}",
            tx_count,
            num_threads
        );
        tracing::debug!(target: "metis::parallel",
            "   spec_id={:?}, chain_id={}, state_clear_flag={}",
            spec_id,
            chain_id,
            state_clear_flag
        );

        // Print transaction hashes for debugging
        for (idx, tx) in transactions.iter().enumerate() {
            let tx_hash = tx.tx().hash();
            tracing::debug!(target: "metis::parallel",
                "   TX[{}]: hash=0x{:x}",
                idx,
                tx_hash
            );
        }

        let pe_start_time = std::time::Instant::now();
        let mut parallel_executor = metis_pe::ParallelExecutor::default();
        tracing::debug!(target: "metis::parallel",
            "🔥 About to call parallel_executor.execute() - THIS IS WHERE PARALLEL EXECUTION HAPPENS"
        );

        let results = parallel_executor.execute(
            StateStorageAdapter::new(db),
            evm_env,
            transactions
                .into_iter()
                .map(|tx| tx.to_tx_env())
                .collect::<Vec<TxEnv>>(),
            NonZeroUsize::new(num_threads).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );

        let pe_duration = pe_start_time.elapsed();
        tracing::info!(target: "metis::parallel",
            "✅ parallel_executor.execute() returned: duration={:?}",
            pe_duration
        );

        // CRITICAL: The parallel executor already sets cumulative_gas_used in each receipt
        // as the cumulative value (including all previous transactions).
        // We should NOT sum all cumulative_gas_used values - that would be wrong!
        // Instead, we use the last receipt's cumulative_gas_used as the total gas.

        // CRITICAL: Merge state changes in transaction order to match serial execution behavior.
        // In serial execution, transactions are executed sequentially, and later transactions
        // see the state changes from earlier transactions. We need to replicate this behavior
        // by merging states in transaction order (not by address).
        //
        // Key insight: Parallel executor returns states for each transaction as if they were
        // executed independently. However, in reality, later transactions may have seen state
        // changes from earlier transactions (due to Block-STM's dependency resolution).
        // We need to merge states in transaction order, where later transactions overwrite
        // earlier transactions for the same address.
        let results_vec = results.map_err(|err| {
            tracing::error!(target: "metis::parallel", "❌ Parallel execution FAILED: {:?}", err);
            BlockExecutionError::Internal(InternalBlockExecutionError::Other(Box::new(err)))
        })?;

        let receipts: Vec<Receipt> = results_vec.iter().map(|r| r.receipt.clone()).collect();

        // Use unified state merge and commit function
        // This ensures consistent state handling across all parallel execution paths
        self.merge_and_commit_parallel_results(&results_vec)?;

        // The total gas used is the cumulative_gas_used of the last receipt
        // (which already includes all previous transactions' gas)
        let total_gas_used = receipts.last().map(|r| r.cumulative_gas_used).unwrap_or(0);

        tracing::info!(target: "metis::parallel",
            "✅ Parallel execution SUCCESSFUL! Total gas used: {}",
            total_gas_used
        );

        // FIX (方案A): Call post_execution() here to ensure block rewards are applied.
        //
        // Problem: In some paths (like execute_block), finish() may not be called,
        // leading to missing block rewards and state root mismatches.
        //
        // Solution: Call post_execution() here with a flag to prevent double application.
        // The flag ensures it's only called once per block execution.
        if !self.context.post_execution_called {
            tracing::debug!(target: "metis::parallel", "Calling post_execution() to apply block rewards");
            self.post_execution()?;
            self.context.mark_post_execution_called();
            tracing::debug!(target: "metis::parallel", "post_execution() completed successfully");
        } else {
            tracing::debug!(target: "metis::parallel", "Skipping post_execution() - already called");
        }

        tracing::info!(target: "metis::parallel",
            "🎉 ParallelBlockExecutor::execute() completed successfully"
        );
        tracing::info!(target: "metis::parallel",
            "   Summary: receipts={}, gas_used={}, blob_gas_used={}",
            receipts.len(),
            total_gas_used,
            total_blob_gas_used
        );

        let result = BlockExecutionResult {
            receipts,
            requests: Requests::default(),
            gas_used: total_gas_used,
            blob_gas_used: total_blob_gas_used,
        };

        tracing::debug!(target: "metis::parallel",
            "✅ Returning BlockExecutionResult from execute()"
        );

        Ok(result)
    }

    fn post_execution(&mut self) -> Result<(), BlockExecutionError> {
        tracing::debug!(target: "metis::parallel", "Starting post-execution balance increments");
        let mut balance_increments = post_block_balance_increments(
            &self.spec,
            self.evm().block(),
            self.executor.ctx.ommers,
            self.executor.ctx.withdrawals.as_deref(),
        );
        tracing::debug!(target: "metis::parallel", "Calculated {} balance increments", balance_increments.len());
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
        tracing::debug!(target: "metis::parallel", "Sorted {} balance increments by address", sorted_increments.len());

        // CRITICAL: Use BTreeMap instead of HashMap to maintain sorted order.
        // BTreeMap preserves the sorted order when iterating, ensuring deterministic
        // state root calculation across all nodes.
        let sorted_balance_increments: std::collections::BTreeMap<_, _> =
            sorted_increments.into_iter().collect();

        // increment balances
        tracing::debug!(target: "metis::parallel", "Applying balance increments to {} addresses", sorted_balance_increments.len());
        self.evm_mut()
            .db_mut()
            .increment_balances(sorted_balance_increments)
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;
        tracing::debug!(target: "metis::parallel", "Balance increments applied successfully");

        Ok(())
    }

    #[allow(dead_code)]
    fn calc_requests(&mut self, receipts: Vec<Receipt>) -> Result<Requests, BlockExecutionError> {
        let evm = self.executor.evm_mut();
        let block = evm.block();
        let requests = if self
            .spec
            .is_prague_active_at_timestamp(block.timestamp.try_into().unwrap_or(u64::MAX))
        {
            // Collect all EIP-6110 deposits
            let deposit_requests =
                eip6110::parse_deposits_from_receipts(self.spec.clone(), &receipts)?;

            let mut requests = Requests::default();

            if !deposit_requests.is_empty() {
                requests.push_request_with_type(eip6110::DEPOSIT_REQUEST_TYPE, deposit_requests);
            }
            let mut system_caller = SystemCaller::new(self.spec.clone());
            requests.extend(system_caller.apply_post_execution_changes(evm)?);
            requests
        } else {
            Requests::default()
        };

        Ok(requests)
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
        // Convert to a vec to count transactions (for logging)
        let tx_vec: Vec<Tx> = transactions.into_iter().collect();
        tracing::info!(target: "metis::parallel",
            "🎯 BatchExecutable::execute_transactions_batch() called with {} transactions",
            tx_vec.len()
        );
        tracing::debug!(target: "metis::parallel",
            "   This is the BatchExecutable trait method - enabling batch parallel execution"
        );

        // Call the existing execute method which handles parallel execution
        let result = self.execute(tx_vec);

        if let Ok(ref res) = result {
            tracing::info!(target: "metis::parallel",
                "✅ BatchExecutable execution completed: receipts={}, gas_used={}",
                res.receipts.len(),
                res.gas_used
            );
        }

        result
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
