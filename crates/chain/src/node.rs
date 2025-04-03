// use reth::{
//     api::NodeTypesWithEngine,
//     builder::{BuilderContext, FullNodeTypes, components::ExecutorBuilder},
// };
// use reth_chainspec::ChainSpec;
// use reth_evm::{execute::{BlockExecutorProvider, Executor}};
// use reth_evm_ethereum::{EthEvmConfig};
// use reth_primitives::EthPrimitives;
// use crate::provider::BlockParallelExecutorProvider;
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
//     type EVM = EthEvmConfig;
//     type Executor = BlockParallelExecutorProvider;
//
//     async fn build_evm(
//         self,
//         ctx: &BuilderContext<Node>,
//     ) -> eyre::Result<(Self::EVM, Self::Executor)> {
//         let evm_config = EthEvmConfig::new(ctx.chain_spec());
//         let executor = BlockParallelExecutorProvider::new(evm_config.clone());
//
//         Ok((evm_config, executor))
//     }
// }
