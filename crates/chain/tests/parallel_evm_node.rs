use metis_chain::provider::ParallelExecutorBuilder;
use reth::{
    builder::{DebugNodeLauncher, EngineNodeLauncher, NodeBuilder, NodeHandle},
    providers::providers::BlockchainProvider,
    tasks::TaskManager,
};
use reth_ethereum::node::EthereumNode;
use reth_node_ethereum::node::EthereumAddOns;
use std::error::Error;

pub mod common;

#[tokio::test]
async fn _test_parallel_evm_node() -> Result<(), Box<dyn Error>> {
    let result = async {
        let tasks = TaskManager::current();

        // create node config
        let node_config = common::node::get_test_node_config();
        let NodeHandle {
            node,
            node_exit_future: _,
        } = NodeBuilder::new(node_config)
            .testing_node(tasks.executor())
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(
                EthereumNode::components().executor(ParallelExecutorBuilder::default()),
            )
            .with_add_ons(EthereumAddOns::default())
            .launch_with_fn(|builder| {
                let engine_launcher = EngineNodeLauncher::new(
                    builder.task_executor().clone(),
                    builder.config().datadir(),
                    Default::default(),
                );
                let launcher = DebugNodeLauncher::new(engine_launcher);
                builder.launch_with(launcher)
            })
            .await?;

        common::node::send_compare_transaction(node).await
    }
    .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("💣 Error Details:");
            eprintln!("{e:#?}");
            if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
                eprintln!("🗂️ IO Error: {io_error}");
            }
            panic!("❌ Test failed due to above error");
        }
    }
}
