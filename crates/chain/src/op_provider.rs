use alloy_consensus::{Block, Header, Receipt, TxType};
use alloy_evm::{
    Database, Evm, EvmEnv, EvmFactory,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockExecutorFactory,
        BlockExecutorFor, CommitChanges, ExecutableTx, OnStateHook,
    },
    op_revm::{OpSpecId, OpTransaction},
    precompiles::PrecompilesMap,
};
use alloy_op_evm::{OpBlockExecutionCtx, OpBlockExecutor, OpEvm, OpEvmFactory};
use metis_primitives::{CfgEnv, DatabaseCommit, ExecutionResult};
use reth::api::NodeTypes;
use reth::builder::{
    BuilderContext, Node, NodeAdapter, NodeComponentsBuilder, components::ExecutorBuilder,
};
use reth::revm::{Inspector, context::TxEnv};
use reth_evm::{ConfigureEvm, ConfigureEngineEvm, EvmEnvFor, ExecutionCtxFor, ExecutableTxIterator,};
use reth_node_api::FullNodeTypes;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::{OpEvmConfig, OpRethReceiptBuilder};
use reth_optimism_primitives::{OpPrimitives, OpReceipt, OpTransactionSigned};
use reth_primitives::SealedBlock;
use reth_primitives_traits::SealedHeader;
use revm::{
    context::result::{ResultAndState},
    database::State,
};

use crate::state::StateStorageAdapter;
use alloy_eips::eip7685::Requests;
use alloy_evm::block::{
    BlockValidationError, InternalBlockExecutionError, state_changes::post_block_balance_increments,
};
use reth::builder::components::{BasicPayloadServiceBuilder, ComponentsBuilder};
use reth_optimism_forks::OpHardforks;
use reth_optimism_node::node::{
    OpAddOns, OpConsensusBuilder, OpEngineValidatorBuilder, OpNetworkBuilder, OpPayloadBuilder,
    OpPoolBuilder,
};
use reth_optimism_node::{OpEngineApiBuilder, OpNode, args::RollupArgs};
use reth_optimism_rpc::eth::OpEthApiBuilder;
use std::{fmt::Debug, num::NonZeroUsize, sync::Arc};
use op_alloy_consensus::EIP1559ParamError;
use op_alloy_rpc_types_engine::OpExecutionData;

/// Optimism-related EVM configuration with the parallel executor.
#[derive(Debug, Clone)]
pub struct OpParallelEvmConfig {
    pub config: OpEvmConfig,
}

impl OpParallelEvmConfig {
    pub fn new(chain_spec: Arc<OpChainSpec>) -> Self {
        Self {
            config: OpEvmConfig::optimism(chain_spec),
        }
    }
}

impl BlockExecutorFactory for OpParallelEvmConfig {
    type EvmFactory = OpEvmFactory;
    type ExecutionCtx<'a> = OpBlockExecutionCtx;
    type Transaction = OpTransactionSigned;
    type Receipt = OpReceipt;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.config.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: OpEvm<&'a mut State<DB>, I, PrecompilesMap>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> impl BlockExecutorFor<'a, Self, DB, I>
    where
        DB: Database + 'a,
        I: Inspector<<Self::EvmFactory as EvmFactory>::Context<&'a mut State<DB>>> + 'a,
    {
        OpParallelBlockExecutor {
            spec: self.config.chain_spec().clone(),
            inner: OpBlockExecutor::new(
                evm,
                ctx,
                self.config.chain_spec().clone(),
                *self.config.executor_factory.receipt_builder(),
            ),
        }
    }
}

impl ConfigureEvm for OpParallelEvmConfig {
    type Primitives = OpPrimitives;
    type Error = <OpEvmConfig as ConfigureEvm>::Error;
    type NextBlockEnvCtx = <OpEvmConfig as ConfigureEvm>::NextBlockEnvCtx;
    type BlockExecutorFactory = Self;
    type BlockAssembler = <OpEvmConfig as ConfigureEvm>::BlockAssembler;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.config.block_assembler()
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnv<OpSpecId>, EIP1559ParamError> {
        self.config.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnv<OpSpecId>, Self::Error> {
        self.config.next_evm_env(parent, attributes)
    }

    fn context_for_block(
        &self,
        block: &SealedBlock<Block<OpTransactionSigned>>,
    ) -> Result<OpBlockExecutionCtx, EIP1559ParamError> {
        self.config.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<OpBlockExecutionCtx, EIP1559ParamError> {
        self.config.context_for_next_block(parent, attributes)
    }
}

impl ConfigureEngineEvm<OpExecutionData> for OpParallelEvmConfig {
    fn evm_env_for_payload(&self, payload: &OpExecutionData) -> EvmEnvFor<Self> {
        self.config.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(&self, payload: &'a OpExecutionData) -> ExecutionCtxFor<'a, Self> {
        self.config.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(&self, payload: &OpExecutionData) -> impl ExecutableTxIterator<Self> {
        self.config.tx_iterator_for_payload(payload)
    }
}

/// Parallel block executor for Optimism.
pub struct OpParallelBlockExecutor<Evm, Spec> {
    spec: Spec,
    inner: OpBlockExecutor<Evm, OpRethReceiptBuilder, Spec>,
}

impl<'db, DB, E, Spec> BlockExecutor for OpParallelBlockExecutor<E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = OpTransaction<TxEnv>>,
    Spec: OpHardforks,
{
    type Transaction = OpTransactionSigned;
    type Receipt = OpReceipt;
    type Evm = E;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.inner.apply_pre_execution_changes()
    }

    fn execute_transaction_with_result_closure(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as Evm>::HaltReason>),
    ) -> Result<u64, BlockExecutionError> {
        self.inner.execute_transaction_with_result_closure(tx, f)
    }

    fn execute_transaction_with_commit_condition(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as Evm>::HaltReason>) -> CommitChanges,
    ) -> Result<Option<u64>, BlockExecutionError> {
        self.inner.execute_transaction_with_commit_condition(tx, f)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn set_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.inner.set_state_hook(hook)
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }

    fn execute_block(
        mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Self::Receipt>, BlockExecutionError>
    where
        Self: Sized,
    {
        self.apply_pre_execution_changes()?;
        self.execute_transactions(transactions)
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<ResultAndState<<Self::Evm as Evm>::HaltReason>, BlockExecutionError> {
        self.inner.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(
        &mut self,
        output: ResultAndState<<Self::Evm as Evm>::HaltReason>,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        self.inner.commit_transaction(output, tx)
    }
}

impl<'db, DB, E, Spec> OpParallelBlockExecutor<E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = OpTransaction<TxEnv>>,
    Spec: OpHardforks,
{
    pub fn execute_transactions(
        &mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<OpReceipt>, BlockExecutionError> {
        // Execute block transactions parallel
        let parallel_execute_result = self.execute(transactions)?;
        let receipts = parallel_execute_result.receipts;
        let requests = parallel_execute_result.requests;

        // Governance reward for full block, ommers...
        self.post_execution()?;

        // Assemble new block execution result, there is no dump receipt
        let results = BlockExecutionResult {
            receipts,
            requests,
            gas_used: parallel_execute_result.gas_used,
        };

        Ok(results)
    }

    pub fn execute(
        &mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<OpReceipt>, BlockExecutionError> {
        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        let state_clear_flag = self.spec.is_spurious_dragon_active_at_block(
            self.evm().block().number.try_into().unwrap_or(u64::MAX),
        );
        let evm_env = EvmEnv::new(CfgEnv::default(), self.evm().block().clone());
        let db = self.evm_mut().db_mut();
        db.set_state_clear_flag(state_clear_flag);
        let mut op_parallel_executor = metis_pe::OpParallelExecutor::default();
        let results = op_parallel_executor.execute(
            StateStorageAdapter::new(db),
            evm_env,
            transactions
                .into_iter()
                .map(|tx| tx.to_tx_env().base)
                .collect::<Vec<TxEnv>>(),
            NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );

        let mut total_gas_used: u64 = 0;
        let receipts = results
            .map_err(|err| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(Box::new(err)))
            })?
            .into_iter()
            .map(|r| {
                self.evm_mut().db_mut().commit(r.state);
                total_gas_used += &r.receipt.cumulative_gas_used;
                match &r.receipt.tx_type {
                    TxType::Legacy => OpReceipt::Legacy(Receipt::from(r.receipt)),
                    TxType::Eip1559 => OpReceipt::Eip1559(Receipt::from(r.receipt)),
                    TxType::Eip2930 => OpReceipt::Eip2930(Receipt::from(r.receipt)),
                    TxType::Eip7702 => OpReceipt::Eip7702(Receipt::from(r.receipt)),
                    TxType::Eip4844 => panic!("don't support eip4844"),
                }
            })
            .collect();

        Ok(BlockExecutionResult {
            receipts,
            gas_used: total_gas_used,
            requests: Requests::default(),
        })
    }

    fn post_execution(&mut self) -> Result<(), BlockExecutionError> {
        let balance_increments =
            post_block_balance_increments::<Header>(&self.spec, self.evm().block(), &[], None);
        // increment balances
        self.evm_mut()
            .db_mut()
            .increment_balances(balance_increments.clone())
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;

        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct OpParallelExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for OpParallelExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = OpChainSpec, Primitives = OpPrimitives>>,
{
    type EVM = OpParallelEvmConfig;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(OpParallelEvmConfig::new(ctx.chain_spec()))
    }
}

#[derive(Debug, Clone)]
pub struct OpParallelNode {
    op_node: OpNode,
}

impl OpParallelNode {
    pub fn new(op_node: OpNode) -> Self {
        Self { op_node }
    }
}

impl NodeTypes for OpParallelNode {
    type Primitives = <OpNode as NodeTypes>::Primitives;
    type ChainSpec = <OpNode as NodeTypes>::ChainSpec;
    // type StateCommitment = <OpNode as NodeTypes>::StateCommitment;
    type Storage = <OpNode as NodeTypes>::Storage;
    type Payload = <OpNode as NodeTypes>::Payload;
}

impl<N> Node<N> for OpParallelNode
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        OpPoolBuilder,
        BasicPayloadServiceBuilder<OpPayloadBuilder>,
        OpNetworkBuilder,
        OpParallelExecutorBuilder,
        OpConsensusBuilder,
    >;

    type AddOns = OpAddOns<
        NodeAdapter<N, <Self::ComponentsBuilder as NodeComponentsBuilder<N>>::Components>,
        OpEthApiBuilder,
        OpEngineValidatorBuilder,
        OpEngineApiBuilder<OpEngineValidatorBuilder>,
    >;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        let RollupArgs {
            disable_txpool_gossip,
            compute_pending_block,
            discovery_v4,
            ..
        } = self.op_node.args;
        ComponentsBuilder::default()
            .node_types::<N>()
            .pool(
                OpPoolBuilder::default()
                    .with_enable_tx_conditional(self.op_node.args.enable_tx_conditional)
                    .with_supervisor(
                        self.op_node.args.supervisor_http.clone(),
                        self.op_node.args.supervisor_safety_level,
                    ),
            )
            .executor(OpParallelExecutorBuilder::default())
            .payload(BasicPayloadServiceBuilder::new(
                OpPayloadBuilder::new(compute_pending_block)
                    .with_da_config(self.op_node.da_config.clone()),
            ))
            .network(OpNetworkBuilder::new(disable_txpool_gossip, !discovery_v4))
            .consensus(OpConsensusBuilder::default())
    }

    fn add_ons(&self) -> Self::AddOns {
        Self::AddOns::builder()
            .with_sequencer(self.op_node.args.sequencer.clone())
            .with_sequencer_headers(self.op_node.args.sequencer_headers.clone())
            .with_da_config(self.op_node.da_config.clone())
            .with_enable_tx_conditional(self.op_node.args.enable_tx_conditional)
            .with_min_suggested_priority_fee(self.op_node.args.min_suggested_priority_fee)
            .build()
    }
}
