use std::num::NonZeroUsize;
use alloy_evm::{eth::{EthBlockExecutor}};

use reth::{
    api::{ConfigureEvm},
    providers::BlockExecutionResult,
    revm::{
        context::{result::ExecutionResult},
        db::{State, states::bundle_state::BundleRetention},
    },
};
use reth_chainspec::ChainSpec;
use reth_evm::{
    Database, Evm, OnStateHook,
    execute::{BlockExecutionError, BlockExecutor, BlockExecutorProvider, Executor},
};
use reth_evm_ethereum::{RethReceiptBuilder};
use reth_primitives::{NodePrimitives, Receipt, Recovered, RecoveredBlock,TransactionSigned};
use std::sync::Arc;
use metis_pe::chain::{PevmChain, PevmEthereum};
use revm::primitives::TxEnv;

pub struct BlockParallelExecutorProvider<F> {
    strategy_factory: F,
    chain_spec: Arc<ChainSpec>,
}

impl<F> BlockParallelExecutorProvider<F> {
    pub const fn new(strategy_factory: F, chain_spec: Arc<ChainSpec>) -> Self {
        Self { strategy_factory, chain_spec }
    }
}

impl<F> Clone for BlockParallelExecutorProvider<F>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            strategy_factory: self.strategy_factory.clone(),
            chain_spec: self.chain_spec.clone(),
        }
    }
}

impl<F> BlockExecutorProvider for BlockParallelExecutorProvider<F>
where
    F: ConfigureEvm + 'static,
{
    type Primitives = F::Primitives;

    type Executor<DB: Database> = ParallelExecutor<F, DB>;

    fn executor<DB>(&self, db: DB) -> Self::Executor<DB>
    where
        DB: Database,
    {
        ParallelExecutor::new(self.strategy_factory.clone(), db, self.chain_spec.clone())
    }
}

pub struct ParallelExecutor<F, DB> {
    /// Block execution strategy.
    pub(crate) strategy_factory: F,
    /// Database.
    pub(crate) db: State<DB>,
    /// Chain spec
    pub chain_spec: Arc<ChainSpec>,
}

impl<F, DB: Database> ParallelExecutor<F, DB> {
    pub fn new(strategy_factory: F, db: DB, chain_spec: Arc<ChainSpec>) -> Self {
        let db = State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build();
        Self {
            strategy_factory,
            db,
            chain_spec,
        }
    }
}

impl<F, DB> Executor<DB> for ParallelExecutor<F, DB>
where
    F: ConfigureEvm,
    DB: Database,
{
    type Primitives = F::Primitives;
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
        self.db
    }

    fn size_hint(&self) -> usize {
        self.db.bundle_state.size_hint()
    }
}

impl<F, DB> ParallelExecutor<F, DB>
where
    F: ConfigureEvm,
    DB: Database,
    <<F as ConfigureEvm>::Primitives as NodePrimitives>::SignedTx: Clone,
{
    fn execute_block(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<u64, BlockExecutionError>
    {
        let mut executor = metis_pe::ParallelExecutor::default();
        let chain_spec = PevmEthereum::mainnet();
        let header = crate::utils::convert_to_alloy_header(&block.header());
        let spec_id = chain_spec.get_block_spec(&header).unwrap();
        let block_env = metis_pe::compat::get_block_env(&header, spec_id);

        let tx_envs = block.transactions_with_sender()
            .map(|(sender, signed_tx)| {
                let tx_env = crate::utils::from_recovered_tx(signed_tx.clone(), sender.clone());
                Ok(tx_env)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let results = executor.execute_revm_parallel(
            &chain_spec,
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


pub struct ParallelBlockExecutor<'a, Evm> {
    /// Inner Ethereum execution strategy.
    pub(crate) inner: EthBlockExecutor<'a, Evm, &'a Arc<ChainSpec>, &'a RethReceiptBuilder>,
}

impl<'db, DB, E> BlockExecutor for ParallelBlockExecutor<'_, E>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv>,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.inner.apply_pre_execution_changes()
    }

    fn execute_transaction_with_result_closure(
        &mut self,
        tx: Recovered<&TransactionSigned>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as Evm>::HaltReason>),
    ) -> Result<u64, BlockExecutionError> {
        self.inner.execute_transaction_with_result_closure(tx, f)
    }

    fn finish(self) -> Result<(Self::Evm, BlockExecutionResult<Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn set_state_hook(&mut self, _hook: Option<Box<dyn OnStateHook>>) {
        self.inner.set_state_hook(_hook)
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }
}

