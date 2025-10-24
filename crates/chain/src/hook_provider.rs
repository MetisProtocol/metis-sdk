use alloy_evm::{
    eth::EthEvmContext,
    precompiles::PrecompilesMap,
    revm::{
        handler::EthPrecompiles,
    },
    EvmFactory,
};
use reth_ethereum::{
    chainspec::{Chain, ChainSpec},
    evm::{
        primitives::{Database, EvmEnv},
        revm::{
            context::{Context, TxEnv},
            context_interface::result::{EVMError, HaltReason},
            inspector::{Inspector, NoOpInspector},
            interpreter::interpreter::EthInterpreter,
            primitives::hardfork::SpecId,
            MainBuilder, MainContext,
        },
        EthEvm, EthEvmConfig,
    },
    node::{
        api::{FullNodeTypes, NodeTypes},
        builder::{components::ExecutorBuilder, BuilderContext},
    },
    EthPrimitives,
};
use std::fmt::Debug;
use metis_hook::evm::MyEvm;

/// Custom EVM configuration.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct MyEvmFactory;

impl EvmFactory for MyEvmFactory {
    type Evm<DB: Database, I: Inspector<EthEvmContext<DB>, EthInterpreter>> =
        EthEvm<DB, I, Self::Precompiles>;
    type Tx = TxEnv;
    type Error<DBError: core::error::Error + Send + Sync + 'static> = EVMError<DBError>;
    type HaltReason = HaltReason;
    type Context<DB: Database> = EthEvmContext<DB>;
    type Spec = SpecId;
    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(&self, db: DB, input: EvmEnv) -> Self::Evm<DB, NoOpInspector> {
        let evm = Context::mainnet()
            .with_db(db)
            .with_cfg(input.cfg_env)
            .with_block(input.block_env)
            .build_mainnet_with_inspector(NoOpInspector {})
            .with_precompiles(EthPrecompiles::default());

        let my_evm = MyEvm::new(evm.ctx, NoOpInspector {});
        // let my_evm = MyEvm(evm);
        // if spec == SpecId::PRAGUE {
        //     evm = evm.with_precompiles(PrecompilesMap::from_static(prague_custom()));
        // }
        EthEvm::new(my_evm, false)
        // my_evm
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<Self::Context<DB>, EthInterpreter>>(
        &self,
        db: DB,
        input: EvmEnv,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        EthEvm::new(self.create_evm(db, input).into_inner().with_inspector(inspector), true)
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct HookExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for HookExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = EthEvmConfig<ChainSpec, MyEvmFactory>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        let evm_config =
            EthEvmConfig::new_with_evm_factory(ctx.chain_spec(), MyEvmFactory::default());
        Ok(evm_config)
    }
}