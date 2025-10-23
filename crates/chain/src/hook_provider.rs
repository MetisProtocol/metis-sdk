use crate::state::StateStorageAdapter;
use alloy_consensus::Header;
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
use metis_primitives::{CfgEnv, ExecutionResult, SpecId, TxEnv, EvmState};
use reth::api::{FullNodeTypes, NodeTypes};
use reth::builder::BuilderContext;
use reth::builder::components::ExecutorBuilder;
use reth::{providers::BlockExecutionResult, revm::db::State};
use reth_chainspec::{ChainSpec, EthChainSpec, Hardforks};
use reth_ethereum_primitives::{Block, EthPrimitives, Receipt, TransactionSigned};
use reth_evm::TransactionEnv;
use reth_evm::block::{ExecutableTx, InternalBlockExecutionError};
use reth_evm::eth::spec::EthExecutorSpec;
use reth_evm::eth::{EthBlockExecutionCtx, EthBlockExecutor, EthBlockExecutorFactory};
use reth_evm::precompiles::PrecompilesMap;
use reth_evm::{
    ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
};
use reth_evm::{EthEvmFactory, Evm, EvmEnv, EvmFactory, NextBlockEnvAttributes};
use reth_evm::{OnStateHook, execute::BlockExecutor};
pub use reth_evm_ethereum::EthEvmConfig;
use reth_evm_ethereum::{EthBlockAssembler, RethReceiptBuilder};
use reth_primitives_traits::{SealedBlock, SealedHeader};
use revm::{DatabaseCommit, context::result::ResultAndState};
use std::convert::Infallible;
use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;
use revm::{handler::Handler, interpreter::interpreter_action::FrameInit};
use revm::handler::{EvmTr, FrameResult, FrameTr};
use revm::{
    Database as RevmDatabase,
    MainContext, MainnetEvm, MainnetContext,
    context::{
        ContextTr,
        result::{EVMError, InvalidTransaction},
    },
    context_interface::{JournalTr, result::HaltReason},
    state::AccountInfo,
};

/// Ethereum-related EVM configuration with the Hook executor.
#[derive(Debug, Clone)]
pub struct HookEthEvmConfig<C = ChainSpec, EvmFactory = EthEvmFactory> {
    pub config: EthEvmConfig<C, EvmFactory>,
}

impl<ChainSpec> HookEthEvmConfig<ChainSpec> {
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            config: EthEvmConfig::new(chain_spec),
        }
    }
}

impl<ChainSpec, EvmF> ConfigureEvm for HookEthEvmConfig<ChainSpec, EvmF>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
    EvmF: EvmFactory<
            Tx: TransactionEnv
                    + FromRecoveredTx<TransactionSigned>
                    + FromTxWithEncoded<TransactionSigned>,
            Spec = SpecId,
            Precompiles = PrecompilesMap,
        > + Clone
        + Debug
        + Send
        + Sync
        + Unpin
        + 'static,
{
    type Primitives = EthPrimitives;
    type Error = Infallible;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = EthBlockExecutorFactory<RethReceiptBuilder, Arc<ChainSpec>, EvmF>;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.config.executor_factory
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        &self.config.block_assembler
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnv, Self::Error> {
        self.config.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnv, Self::Error> {
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

impl ConfigureEngineEvm<ExecutionData> for HookEthEvmConfig<ChainSpec>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
{
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> EvmEnvFor<Self> {
        self.config.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(&self, payload: &'a ExecutionData) -> ExecutionCtxFor<'a, Self> {
        self.config.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(&self, payload: &ExecutionData) -> impl ExecutableTxIterator<Self> {
        self.config.tx_iterator_for_payload(payload)
    }
}

/// Hook block executor for Ethereum.
pub struct HookBlockExecutor<'a, Evm, Spec> {
    /// Reference to the specification object.
    spec: Spec,
    /// Eth original executor.
    executor: EthBlockExecutor<'a, Evm, Spec, RethReceiptBuilder>,
}

impl<'db, DB, E, Spec> BlockExecutor for HookBlockExecutor<'_, E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv>,
    Spec: EthExecutorSpec + Clone,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.executor.apply_pre_execution_changes()
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
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

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<ResultAndState<<Self::Evm as Evm>::HaltReason>, BlockExecutionError> {
        let mut evm = build_evm(&mut db, self.evm_env.clone());
        let result = {
            evm.set_tx(tx.clone());
            #[cfg(feature = "compiler")]
            let mut t = DirectFeeHandler::new(self.worker.clone());
            #[cfg(not(feature = "compiler"))]
            let mut t = DirectFeeHandler::default();
            t.run(&mut evm)
        };
        self.executor.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(
        &mut self,
        output: ResultAndState<<Self::Evm as Evm>::HaltReason>,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        self.executor.commit_transaction(output, tx)
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct HookExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for HookExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = HookEthEvmConfig;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(HookEthEvmConfig::new(ctx.chain_spec()))
    }
}

#[inline]
pub(crate) fn build_evm<DB: RevmDatabase>(db: DB, evm_env: EvmEnv) -> MainnetEvm<MainnetContext<DB>> {
    MainnetContext::mainnet()
        .with_db(db)
        .with_cfg(evm_env.cfg_env)
        .with_block(evm_env.block_env)
        .build_mainnet()
}

pub struct DirectFeeHandler<EVM> {
    _phantom: core::marker::PhantomData<EVM>,
    #[cfg(feature = "compiler")]
    worker: Arc<metis_vm::ExtCompileWorker>,
}

impl<EVM> Handler for DirectFeeHandler<EVM>
where
    EVM: EvmTr<
            Context: ContextTr<Journal: JournalTr<State = EvmState>>,
            Frame: FrameTr<FrameInit = FrameInit, FrameResult = FrameResult>,
        >,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as RevmDatabase>::Error, InvalidTransaction>;
    type HaltReason = HaltReason;

    fn reward_beneficiary(
        &self,
        _evm: &mut Self::Evm,
        _exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        // Skip beneficiary reward
        Ok(())
    }
}

#[cfg(not(feature = "compiler"))]
impl<EVM> Default for DirectFeeHandler<EVM> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

#[cfg(feature = "compiler")]
impl<EVM> Default for DirectFeeHandler<EVM> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            worker: Arc::new(metis_vm::ExtCompileWorker::disable()),
        }
    }
}

#[cfg(feature = "compiler")]
impl<EVM> DirectFeeHandler<EVM> {
    #[inline]
    pub fn new(worker: Arc<metis_vm::ExtCompileWorker>) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            worker,
        }
    }
}