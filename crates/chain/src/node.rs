// use alloy_evm::{
//     EthEvm, EthEvmFactory,
//     block::{BlockExecutorFactory, BlockExecutorFor},
//     eth::{EthBlockExecutionCtx, EthBlockExecutor}};
// use reth::{
//     api::{ConfigureEvm, NodeTypesWithEngine},
//     builder::{BuilderContext, FullNodeTypes, components::ExecutorBuilder},
//     revm::{
//         db::{State},
//         primitives::hardfork::SpecId,
//     },
// };
// use reth_chainspec::ChainSpec;
// use reth_evm::{
//     Database, EvmEnv, InspectorFor, NextBlockEnvAttributes,
// };
// use reth_evm_ethereum::{EthBlockAssembler, EthEvmConfig};
// use reth_primitives::{
//     EthPrimitives, Header, NodePrimitives, Receipt,  SealedBlock,
//     SealedHeader, TransactionSigned,
// };
// use crate::provider::{BlockParallelExecutorProvider, ParallelBlockExecutor};
//
// /// A custom executor builder
// #[derive(Debug, Default, Clone, Copy)]
// #[non_exhaustive]
// pub struct ParallelExecutorBuilder;
//
// impl<Types, Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
// where
//     Types: NodeTypesWithEngine<ChainSpec = ChainSpec, Primitives = EthPrimitives>,
//     Node: FullNodeTypes<Types = Types>,
// {
//     type EVM = ParallelEvmConfig;
//     type Executor = BlockParallelExecutorProvider<Self::EVM>;
//
//     async fn build_evm(
//         self,
//         ctx: &BuilderContext<Node>,
//     ) -> eyre::Result<(Self::EVM, Self::Executor)> {
//         let chain_spec = ctx.chain_spec();
//         let evm_config = ParallelEvmConfig {
//             inner: EthEvmConfig::new(chain_spec.clone()),
//         };
//         let executor = BlockParallelExecutorProvider::new(
//             evm_config.clone(),
//             chain_spec,
//         );
//         Ok((evm_config, executor))
//     }
// }
//
// #[derive(Debug, Clone)]
// pub struct ParallelEvmConfig {
//     inner: EthEvmConfig,
// }
//
// impl ParallelEvmConfig {
//     pub fn chain_spec(&self) -> &ChainSpec {
//         self.inner.chain_spec()
//     }
// }
//
// impl BlockExecutorFactory for ParallelEvmConfig {
//     type EvmFactory = EthEvmFactory;
//     type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
//     type Transaction = TransactionSigned;
//     type Receipt = Receipt;
//
//     fn evm_factory(&self) -> &Self::EvmFactory {
//         self.inner.evm_factory()
//     }
//
//     fn create_executor<'a, DB, I>(
//         &'a self,
//         evm: EthEvm<&'a mut State<DB>, I>,
//         ctx: EthBlockExecutionCtx<'a>,
//     ) -> impl BlockExecutorFor<'a, Self, DB, I>
//     where
//         DB: Database + 'a,
//         I: InspectorFor<Self, &'a mut State<DB>> + 'a,
//     {
//         ParallelBlockExecutor {
//             inner: EthBlockExecutor::new(
//                 evm,
//                 ctx,
//                 self.inner.chain_spec(),
//                 self.inner.executor_factory.receipt_builder(),
//             ),
//         }
//     }
// }
//
// impl ConfigureEvm for ParallelEvmConfig {
//     type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;
//     type Error = <EthEvmConfig as ConfigureEvm>::Error;
//     type NextBlockEnvCtx = <EthEvmConfig as ConfigureEvm>::NextBlockEnvCtx;
//     type BlockExecutorFactory = Self;
//     type BlockAssembler = EthBlockAssembler<ChainSpec>;
//
//     fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
//         self
//     }
//
//     fn block_assembler(&self) -> &Self::BlockAssembler {
//         self.inner.block_assembler()
//     }
//
//     fn evm_env(&self, header: &Header) -> EvmEnv<SpecId> {
//         self.inner.evm_env(header)
//     }
//
//     fn next_evm_env(
//         &self,
//         parent: &Header,
//         attributes: &NextBlockEnvAttributes,
//     ) -> Result<EvmEnv<SpecId>, Self::Error> {
//         self.inner.next_evm_env(parent, attributes)
//     }
//
//     fn context_for_block<'a>(&self, block: &'a SealedBlock) -> EthBlockExecutionCtx<'a> {
//         self.inner.context_for_block(block)
//     }
//
//     fn context_for_next_block(
//         &self,
//         parent: &SealedHeader,
//         attributes: Self::NextBlockEnvCtx,
//     ) -> EthBlockExecutionCtx<'_> {
//         self.inner.context_for_next_block(parent, attributes)
//     }
// }
