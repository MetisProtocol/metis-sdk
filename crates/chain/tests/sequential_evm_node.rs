use reth::{
    builder::{NodeBuilder, NodeHandle},
    tasks::TaskManager,
};
use reth_ethereum::node::EthereumNode;

pub mod common;

#[tokio::test]
async fn test_custom_evm_node() -> eyre::Result<()> {
    let tasks = TaskManager::current();
    let node_config = common::node::get_test_node_config();
    let NodeHandle {
        node,
        node_exit_future: _,
    } = NodeBuilder::new(node_config)
        .testing_node(tasks.executor())
        .node(EthereumNode::default())
        .launch()
        .await?;

    let _ = common::node::send_compare_transaction(node).await;
    Ok(())
}
