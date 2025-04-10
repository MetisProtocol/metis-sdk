use std::default::Default;
use alloy_genesis::Genesis;
use metis_chain::provider::ParallelExecutorBuilder;
use metis_pe::EvmAccount;
use reth::builder::{DebugNode, DebugNodeLauncher, EngineNodeLauncher, LaunchNode, Node, NodeAdapter, NodeBuilder, NodeBuilderWithComponents};
use reth::builder::components::BasicPayloadServiceBuilder;
use reth::dirs::DataDirPath;
use reth::revm::primitives::alloy_primitives::U160;
use reth::revm::primitives::{Address, U256};
use reth::tasks::TaskManager;
use reth_ethereum::chainspec::{Chain, ChainSpec};
use reth_ethereum::node::{
    EthereumNode,
    core::{args::RpcServerArgs, node_config::NodeConfig},
    node::EthereumAddOns,
};
use reth_ethereum::provider::db::DatabaseEnv;
use reth_ethereum::provider::db::test_utils::{TempDatabase, create_test_rw_db_with_path};
use reth_tracing::{RethTracer, Tracer};
use std::sync::Arc;
use reth::chainspec::EthereumChainSpecParser;
use reth::{args::RessArgs, cli::Cli, ress::install_ress_subprotocol};
use reth::builder::NodeHandle;
use clap::Parser;
use reth_ethereum::provider::db::mdbx::EnvironmentKind::Default;
use tracing::info;

const GIGA_GAS: u64 = 1_000_000_000;
const RAW_TRANSFER_GAS_LIMIT: u64 = 21_000;
const ERROR_TEMPDIR: &str = "could not create a temporary directory";
const DATA_DIR_PREFIX: &str = "reth-test-";

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let _guard = RethTracer::new().init()?;

    let tasks = TaskManager::current();

    // create a custom chain spec
    let spec = ChainSpec::builder()
        .chain(Chain::mainnet())
        .genesis(Genesis::default())
        .london_activated()
        .paris_activated()
        .shanghai_activated()
        .cancun_activated()
        .build();

    let node_config = NodeConfig::test()
        .with_rpc(RpcServerArgs::default().with_http())
        .with_chain(spec);

    let db = crate_database(node_config.chain.clone());

    let provider = ParallelExecutorBuilder::default();

    let builder = NodeBuilder::new(node_config)
        .with_database(db)
        .with_launch_context(tasks.executor());

    let handler = builder
        .with_types::<EthereumNode>()
        .with_components(
            EthereumNode::components()
                .executor(provider)
                .payload(BasicPayloadServiceBuilder::default()),
        )
        .with_add_ons(EthereumAddOns::default())
        .launch()
        .await?;

    println!("Node started");

    handler.node_exit_future.await
}

fn crate_database(chain: Arc<ChainSpec>) -> Arc<TempDatabase<DatabaseEnv>> {
    let temp_path_builder = tempfile::Builder::new()
        .prefix(DATA_DIR_PREFIX)
        .rand_bytes(8)
        .tempdir();
    let temp_path_buf = temp_path_builder.expect(ERROR_TEMPDIR).into_path();
    let path = reth::core::dirs::MaybePlatformPath::<DataDirPath>::from(temp_path_buf);
    let data_dir_args = reth::args::DatadirArgs {
        datadir: path.clone(),
        ..Default::default()
    };
    let data_dir = path.unwrap_or_chain_default(chain.chain, data_dir_args);
    create_test_rw_db_with_path(data_dir.db())
}

pub fn mock_account(idx: usize) -> (Address, EvmAccount) {
    let address = Address::from(U160::from(idx));
    let account = EvmAccount {
        // Filling half full accounts to have enough tokens for tests without worrying about
        // the corner case of balance not going beyond [U256::MAX].
        balance: U256::MAX.div_ceil(U256::from(2)),
        nonce: 1,
        ..EvmAccount::default()
    };
    (address, account)
}
