use alloy_genesis::Genesis;
use metis_chain::provider::ParallelExecutorBuilder;
use metis_pe::{EvmAccount, InMemoryStorage};
use reth::builder::NodeBuilder;
use reth::builder::components::BasicPayloadServiceBuilder;
use reth::revm::primitives::alloy_primitives::U160;
use reth::revm::primitives::{Address, U256};
use reth::tasks::TaskManager;
use reth_ethereum::chainspec::{Chain, ChainSpec};
use reth_ethereum::node::{
    EthereumNode,
    core::{args::RpcServerArgs, node_config::NodeConfig},
    node::EthereumAddOns,
};
use reth_tracing::{RethTracer, Tracer};

const GIGA_GAS: u64 = 1_000_000_000;
pub const RAW_TRANSFER_GAS_LIMIT: u64 = 21_000;

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

    let db = crate_database();

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

fn crate_database() -> InMemoryStorage {
    let block_size = (GIGA_GAS as f64 / RAW_TRANSFER_GAS_LIMIT as f64).ceil() as usize;
    // Skip the built-in precompiled contracts addresses.
    const START_ADDRESS: usize = 1000;
    const MINER_ADDRESS: usize = 0;
    InMemoryStorage::new(
        std::iter::once(MINER_ADDRESS)
            .chain(START_ADDRESS..START_ADDRESS + block_size)
            .map(mock_account)
            .collect(),
        Default::default(),
        Default::default(),
    )
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
