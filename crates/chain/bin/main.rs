#![allow(missing_docs)]

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

use metis_chain::provider::ParallelExecutorBuilder;
use reth::cli::Cli;
use reth_node_ethereum::EthereumNode;
use reth_node_ethereum::node::EthereumAddOns;
use tracing::info;

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    if let Err(err) = Cli::parse_args().run(async move |builder, _| {
        info!(target: "reth::cli", "Launching node");
        let handle = builder
            // use the default ethereum node types
            .with_types::<EthereumNode>()
            // Configure the components of the node
            // use default ethereum components but use our custom pool
            .with_components(
                EthereumNode::components().executor(ParallelExecutorBuilder::default()),
            )
            .with_add_ons(EthereumAddOns::default())
            .launch()
            .await?;

        handle.wait_for_node_exit().await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
