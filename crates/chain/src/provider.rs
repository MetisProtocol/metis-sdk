use alloy_evm::eth::EthBlockExecutor;
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
use reth_evm_ethereum::{EthEvmConfig, RethReceiptBuilder};
use reth_primitives::{
    EthPrimitives, NodePrimitives, Receipt, Recovered, RecoveredBlock, TransactionSigned,
};
use std::sync::Arc;
use metis_pe::chain::{PevmChain, PevmEthereum};
use revm::primitives::TxEnv;

pub struct BlockParallelExecutorProvider {
    strategy_factory: EthEvmConfig,
}

impl BlockParallelExecutorProvider {
    pub const fn new(strategy_factory: EthEvmConfig) -> Self {
        Self { strategy_factory }
=======
pub struct BlockParallelExecutorProvider<F> {
    strategy_factory: F,
    chain_spec: Arc<ChainSpec>,
}

impl<F> BlockParallelExecutorProvider<F> {
    pub const fn new(strategy_factory: F, chain_spec: Arc<ChainSpec>) -> Self {
        Self { strategy_factory, chain_spec }
>>>>>>> 1e3acca (execute block)
    }
}

impl Clone for BlockParallelExecutorProvider {
    fn clone(&self) -> Self {
        Self {
            strategy_factory: self.strategy_factory.clone(),
            chain_spec: self.chain_spec.clone(),
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
        ParallelExecutor::new(self.strategy_factory.clone(), db, self.chain_spec.clone())
    }
}

pub struct ParallelExecutor<DB> {
    /// Block execution strategy.
    pub(crate) strategy_factory: EthEvmConfig,
    /// Database.
    pub(crate) db: State<DB>,
    /// Chain spec
    pub chain_spec: Arc<ChainSpec>,
}

<<<<<<< HEAD
impl<DB: Database> ParallelExecutor<DB> {
    pub fn new(strategy_factory: EthEvmConfig, db: DB) -> Self {
=======
impl<F, DB: Database> ParallelExecutor<F, DB> {
    pub fn new(strategy_factory: F, db: DB, chain_spec: Arc<ChainSpec>) -> Self {
>>>>>>> 1e3acca (execute block)
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

impl<DB> Executor<DB> for ParallelExecutor<DB>
where
<<<<<<< HEAD
    DB: Database,
=======
    F: ConfigureEvm,
<<<<<<< HEAD
    DB: Database + Storage + Send + Sync,
>>>>>>> 1a864d8 (use the same lib)
=======
    DB: Database,
>>>>>>> 38c5225 (convert txenv)
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
<<<<<<< HEAD
        // TODO(fk): change this code to the parallel execution based on the `metis-pe` crate.
        for tx in block.transactions_recovered() {
            let _tx_env = self.strategy_factory.tx_env(tx);
            strategy.execute_transaction(tx)?;
        }
=======
        self.execute_block(block)?;
>>>>>>> 1e3acca (execute block)
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
        let header = convert_to_alloy_header(&block.header());
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


<<<<<<< HEAD
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
<<<<<<< HEAD
        let evm_config = EthEvmConfig::new(ctx.chain_spec());
        let executor = BlockParallelExecutorProvider::new(evm_config.clone());

=======
        let chain_spec = ctx.chain_spec();
        let evm_config = ParallelEvmConfig {
            inner: EthEvmConfig::new(chain_spec.clone()),
        };
        let executor = BlockParallelExecutorProvider::new(
            evm_config.clone(),
            chain_spec,
        );
>>>>>>> 1e3acca (execute block)
        Ok((evm_config, executor))
    }
}

<<<<<<< HEAD
=======
#[derive(Debug, Clone)]
pub struct ParallelEvmConfig {
    inner: EthEvmConfig,
}

impl ParallelEvmConfig {
    pub fn chain_spec(&self) -> &ChainSpec {
        self.inner.chain_spec()
    }
}

impl BlockExecutorFactory for ParallelEvmConfig {
    type EvmFactory = EthEvmFactory;
    type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
    type Transaction = TransactionSigned;
    type Receipt = Receipt;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: EthEvm<&'a mut State<DB>, I>,
        ctx: EthBlockExecutionCtx<'a>,
    ) -> impl BlockExecutorFor<'a, Self, DB, I>
    where
        DB: Database + 'a,
        I: InspectorFor<Self, &'a mut State<DB>> + 'a,
    {
        ParallelBlockExecutor {
            inner: EthBlockExecutor::new(
                evm,
                ctx,
                self.inner.chain_spec(),
                self.inner.executor_factory.receipt_builder(),
            ),
        }
    }
}

impl ConfigureEvm for ParallelEvmConfig {
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;
    type Error = <EthEvmConfig as ConfigureEvm>::Error;
    type NextBlockEnvCtx = <EthEvmConfig as ConfigureEvm>::NextBlockEnvCtx;
    type BlockExecutorFactory = Self;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(&self, header: &Header) -> EvmEnv<SpecId> {
        self.inner.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnv<SpecId>, Self::Error> {
        self.inner.next_evm_env(parent, attributes)
    }

    fn context_for_block<'a>(&self, block: &'a SealedBlock) -> EthBlockExecutionCtx<'a> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> EthBlockExecutionCtx<'_> {
        self.inner.context_for_next_block(parent, attributes)
    }
}

>>>>>>> 1e3acca (execute block)
=======
>>>>>>> 38c5225 (convert txenv)
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

fn convert_to_alloy_header<H>(raw: &H) -> alloy_rpc_types_eth::Header
where
    H: Into<reth_primitives::Header>
{
    let header: reth_primitives::Header = raw.into();
    let inner = alloy_consensus::Header {
        parent_hash: header.parent_hash,
        ommers_hash: header.ommers_hash,
        beneficiary: header.beneficiary,
        state_root: header.state_root,
        transactions_root: header.transactions_root,
        receipts_root: header.receipts_root,
        logs_bloom: header.logs_bloom,
        difficulty: header.difficulty,
        number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        extra_data: header.extra_data.clone(),
        mix_hash: header.mix_hash,
        nonce: header.nonce,
        base_fee_per_gas: header.base_fee_per_gas,
        withdrawals_root: header.withdrawals_root,
        blob_gas_used: header.blob_gas_used,
        excess_blob_gas: header.excess_blob_gas,
        parent_beacon_block_root: header.parent_beacon_block_root,
        requests_hash: header.requests_hash,
    };

    alloy_rpc_types_eth::Header::new(inner)
}