use crate::state::StateStorageAdapter;
use alloy_evm::eth::EthBlockExecutor;
use metis_pe::Storage;
use metis_pe::chain::{Chain, Ethereum};
use reth::builder::rpc::EngineValidatorBuilder;
use reth::{
    api::{ConfigureEvm, NodeTypesWithEngine},
    builder::{BuilderContext, FullNodeTypes, components::ExecutorBuilder},
    providers::BlockExecutionResult,
    revm::{
        context::{TxEnv, result::ExecutionResult},
        db::{State, states::bundle_state::BundleRetention},
    },
};
use reth_chainspec::{ChainSpec, EthChainSpec};
use reth_evm::{
    Database, Evm, OnStateHook,
    execute::{BlockExecutionError, BlockExecutor, BlockExecutorProvider, Executor},
};
use reth_evm_ethereum::{EthEvmConfig, RethReceiptBuilder};
use reth_primitives::{
    EthPrimitives, NodePrimitives, Receipt, Recovered, RecoveredBlock, TransactionSigned,
};
use std::num::NonZeroUsize;
use std::sync::Arc;

pub struct BlockParallelExecutorProvider {
    strategy_factory: EthEvmConfig,
}

impl BlockParallelExecutorProvider {
    pub const fn new(strategy_factory: EthEvmConfig) -> Self {
        Self { strategy_factory }
    }
}

impl Clone for BlockParallelExecutorProvider {
    fn clone(&self) -> Self {
        Self {
            strategy_factory: self.strategy_factory.clone(),
        }
    }
}

impl BlockExecutorProvider for BlockParallelExecutorProvider {
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;

    type Executor<DB: Database> = ParallelExecutor<DB>;

    fn executor<DB>(&self, db: DB) -> Self::Executor<DB>
    where
        DB: Database,
    {
        ParallelExecutor::new(self.strategy_factory.clone(), db)
    }
}

pub struct ParallelExecutor<DB> {
    /// Block execution strategy.
    pub(crate) strategy_factory: EthEvmConfig,
    /// Database.
    pub(crate) db: StateStorageAdapter<DB>,
}

impl<DB: Database> ParallelExecutor<DB> {
    pub fn new(strategy_factory: EthEvmConfig, db: StateStorageAdapter<DB>) -> Self {
        let db = State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build();
        let adapter = StateStorageAdapter::new(db);
        Self {
            strategy_factory,
            db: adapter,
        }
    }
}

impl<DB> Executor<DB> for ParallelExecutor<DB>
where
    DB: Database + Storage + Send + Sync,
{
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;
    type Error = BlockExecutionError;

    fn execute_one(
        &mut self,
        block: &RecoveredBlock<<Self::Primitives as NodePrimitives>::Block>,
    ) -> Result<BlockExecutionResult<<Self::Primitives as NodePrimitives>::Receipt>, Self::Error>
    {
        let db = &mut self.db;
        let mut strategy = self.strategy_factory.executor_for_block(db, block);
        strategy.apply_pre_execution_changes()?;
        self.execute_block(block)?;
        let result = strategy.apply_post_execution_changes()?;

        self.db.merge_transitions(BundleRetention::Reverts);

        Ok(result)
    }

    fn execute_one_with_state_hook<H>(
        &mut self,
        block: &RecoveredBlock<<Self::Primitives as NodePrimitives>::Block>,
        state_hook: H,
    ) -> Result<BlockExecutionResult<<Self::Primitives as NodePrimitives>::Receipt>, Self::Error>
    where
        H: OnStateHook + 'static,
    {
        let db = &mut self.db;
        let mut strategy = self
            .strategy_factory
            .executor_for_block(db, block)
            .with_state_hook(Some(Box::new(state_hook)));

        strategy.apply_pre_execution_changes()?;
        self.execute_block(block)?;
        let result = strategy.apply_post_execution_changes()?;

        self.db.merge_transitions(BundleRetention::Reverts);

        Ok(result)
    }

    fn into_state(self) -> State<DB> {
        self.db.state.into_inner().unwrap()
    }

    fn size_hint(&self) -> usize {
        self.db.state.lock().unwrap().bundle_state.size_hint()
    }
}

impl<DB> ParallelExecutor<DB>
where
    DB: Database + Storage + Send + Sync,
{
    fn execute_block(
        &mut self,
        block: &RecoveredBlock<<Self::Primitives as NodePrimitives>::Block>,
    ) -> Result<u64, BlockExecutionError> {
        let mut executor = metis_pe::ParallelExecutor::default();
        let eth_chain = Ethereum::mainnet();
        let chain_spec = self.strategy_factory.chain_spec();
        let spec_id = eth_chain.get_block_spec(block.header()).unwrap();
        let block_env = metis_pe::compat::get_block_env(&block.header(), spec_id);
        let tx_envs = block
            .transactions_with_sender()
            .map(|(_, signed_tx)| signed_tx.tx_env(block_env.clone()))
            .collect::<Result<Vec<_>, _>>()?;

        let results = executor.execute_revm_parallel(
            &chain_spec.as_ref(),
            &self.db,
            spec_id,
            block_env,
            tx_envs,
            NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );

        let mut cumulative_gas_used = 0;
        for result in results.unwrap() {
            cumulative_gas_used += result.receipt.cumulative_gas_used
        }

        Ok(cumulative_gas_used)
    }
}

/// A custom executor builder
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl<Types, Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
where
    Types: NodeTypesWithEngine<ChainSpec = ChainSpec, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
{
    type EVM = EthEvmConfig;
    type Executor = BlockParallelExecutorProvider;

    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let evm_config = EthEvmConfig::new(ctx.chain_spec());
        let executor = BlockParallelExecutorProvider::new(evm_config.clone());

        Ok((evm_config, executor))
    }
}
