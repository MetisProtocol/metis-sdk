use reth::{
    builder::{NodeBuilder, NodeHandle},
    tasks::TaskManager,
};
use reth_ethereum::node::{
    EthereumNode,
    core::{args::RpcServerArgs, node_config::NodeConfig},
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

    common::send_compare_transaction(node).await
}
