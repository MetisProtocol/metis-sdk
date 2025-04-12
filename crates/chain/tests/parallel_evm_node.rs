use metis_chain::provider::ParallelExecutorBuilder;
use reth::builder::Node;
use reth::{
    builder::{NodeBuilder, NodeHandle},
    tasks::TaskManager,
};
use reth_ethereum::node::EthereumNode;
use reth_node_ethereum::node::EthereumAddOns;
use std::error::Error;

pub mod common;

#[tokio::test]
async fn test_custom_dev_node() -> Result<(), Box<dyn Error>> {
    let tasks = TaskManager::current();

    // create node config
    let node_config = common::get_test_node_config();

    let parallel_executor = ParallelExecutorBuilder::default();
    let default_node = EthereumNode::default();
    let components_builder = default_node
        .components_builder()
        .executor(parallel_executor);

    let NodeHandle {
        node,
        node_exit_future: _,
    } = NodeBuilder::new(node_config)
        .testing_node(tasks.executor())
        .with_types::<EthereumNode>()
        .with_components(components_builder)
        .with_add_ons(EthereumAddOns::default())
        .launch()
        .await?;

    common::send_compare_transaction(node).await
}
