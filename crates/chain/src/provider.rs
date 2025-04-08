use alloy_evm::block::BlockExecutionError;
use alloy_evm::{Database, IntoTxEnv};
use metis_primitives::TxEnv;
use reth::api::{FullNodeTypes, NodeTypes};
use reth::builder::BuilderContext;
use reth::builder::components::ExecutorBuilder;
use reth::primitives::EthPrimitives;
use reth::{
    api::ConfigureEvm,
    providers::BlockExecutionResult,
    revm::db::{State, states::bundle_state::BundleRetention},
};
use reth_chainspec::{ChainSpec, EthChainSpec};
use reth_evm::{
    OnStateHook,
    execute::{BlockExecutor, BlockExecutorProvider, Executor},
};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::{NodePrimitives, RecoveredBlock};
use std::fmt::Debug;
use std::num::NonZeroUsize;

use crate::state::StateStorageAdapter;

#[derive(Debug)]
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
        let state_db = State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build();
        ParallelExecutor::new(self.strategy_factory.clone(), state_db)
    }
}

pub struct ParallelExecutor<DB> {
    strategy_factory: EthEvmConfig,
    db: State<DB>,
}

impl<DB> ParallelExecutor<DB> {
    pub fn new(strategy_factory: EthEvmConfig, db: State<DB>) -> Self {
        Self {
            strategy_factory,
            db,
        }
    }
}

impl<DB> Executor<DB> for ParallelExecutor<DB>
where
    DB: Database,
{
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;
    type Error = BlockExecutionError;

    fn execute_one(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<
        BlockExecutionResult<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Receipt>,
        Self::Error,
    > {
        self.execute_block(block)?;
        // TODO(fk): use the parallel executor result instead of the strategy EthBlockExecutor result.
        let result = BlockExecutionResult::default();
        self.db.merge_transitions(BundleRetention::Reverts);
        Ok(result)
    }

    fn execute_one_with_state_hook<F>(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
        state_hook: F,
    ) -> Result<
        BlockExecutionResult<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Receipt>,
        Self::Error,
    >
    where
        F: OnStateHook + 'static,
    {
        {
            let mut strategy = self
                .strategy_factory
                .executor_for_block(&mut self.db, block)
                .with_state_hook(Some(Box::new(state_hook)));
            strategy.apply_pre_execution_changes()?;
        }
        self.execute_block(block)?;
        // TODO(fk): use the parallel executor result instead of the strategy EthBlockExecutor result.
        let result = BlockExecutionResult::default();
        self.db.merge_transitions(BundleRetention::Reverts);
        Ok(result)
    }

    fn into_state(self) -> State<DB> {
        self.db
    }

    fn size_hint(&self) -> usize {
        self.db.bundle_state.size_hint()
    }
}

impl<DB> ParallelExecutor<DB>
where
    DB: Database,
{
    fn execute_block(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<u64, BlockExecutionError> {
        let mut executor = metis_pe::ParallelExecutor::default();
        let env = self.strategy_factory.evm_env(block.header());
        let chain_spec = self.strategy_factory.chain_spec();
        let spec_id = *env.spec_id();
        let block_env = env.block_env;

        let tx_envs = block
            .transactions_recovered()
            .map(|recover_tx| recover_tx.into_tx_env())
            .collect::<Vec<TxEnv>>();

        let chain_id = chain_spec.chain_id();
        let chain = metis_pe::chain::Ethereum::custom(chain_id);
        let results = executor.execute_revm_parallel(
            &chain,
            StateStorageAdapter::new(&mut self.db),
            spec_id,
            block_env,
            tx_envs,
            NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );
        Ok(results
            .unwrap()
            .into_iter()
            .map(|r| r.receipt.cumulative_gas_used)
            .sum())
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
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
