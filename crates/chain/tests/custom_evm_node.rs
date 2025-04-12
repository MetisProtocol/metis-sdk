use alloy_primitives::{B256, b256, hex};
use futures_util::StreamExt;
use reth::{
    builder::{NodeBuilder, NodeHandle},
    rpc::api::EngineEthApiClient,
    tasks::TaskManager,
};
use reth_ethereum::{
    node::{
        EthereumNode,
        core::{args::RpcServerArgs, node_config::NodeConfig},
    },
    provider::CanonStateSubscriptions,
};
use std::error::Error;

pub mod common;

#[tokio::test]
async fn test_custom_dev_node() -> Result<(), Box<dyn Error>> {
    let tasks = TaskManager::current();

    // create node config
    let node_config = NodeConfig::test()
        .dev()
        .with_rpc(RpcServerArgs::default().with_http())
        .with_chain(common::custom_chain());

    let NodeHandle {
        node,
        node_exit_future: _,
    } = NodeBuilder::new(node_config)
        .testing_node(tasks.executor())
        .node(EthereumNode::default())
        .launch()
        .await?;

    let mut notifications = node.provider.canonical_state_stream();

    // submit tx through rpc
    let raw_tx = hex!(
        "02f876820a28808477359400847735940082520894ab0840c0e43688012c1adb0f5e3fc665188f83d28a029d394a5d630544000080c080a0a044076b7e67b5deecc63f61a8d7913fab86ca365b344b5759d1fe3563b4c39ea019eab979dd000da04dfc72bb0377c092d30fd9e1cab5ae487de49586cc8b0090"
    );
    let hash = EngineEthApiClient::<B256, B256>::send_raw_transaction(
        &node.engine_http_client(),
        raw_tx.into(),
    )
    .await?;
    let expected = b256!("0xb1c6512f4fc202c04355fbda66755e0e344b152e633010e8fd75ecec09b63398");

    println!("submitted transaction: {hash}");
    println!("expected transaction: {expected}");

    let head = notifications.next().await.unwrap();
    let tx = &head.tip().body().transactions().next().unwrap();
    println!("notification transaction {:?}", tx.as_ref());

    println!("mined transaction: {hash}");
    Ok(())
}
