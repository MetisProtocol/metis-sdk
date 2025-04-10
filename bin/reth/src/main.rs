#![allow(missing_docs)]

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

use clap::Parser;
use metis_chain::provider::ParallelExecutorBuilder;
use reth::{args::RessArgs, cli::Cli, ress::install_ress_subprotocol};
use reth_ethereum_cli::chainspec::EthereumChainSpecParser;
use reth_node_builder::{Node, NodeHandle};
use reth_node_ethereum::EthereumNode;
use reth_node_ethereum::node::EthereumAddOns;
use tracing::info;

fn main() {
    reth_cli_util::sigsegv_handler::install();
    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    if let Err(err) =
        Cli::<EthereumChainSpecParser, RessArgs>::parse().run(async move |builder, ress_args| {
            info!(target: "reth::cli", "Launching node");
            let node = EthereumNode::default();
            let provider = ParallelExecutorBuilder::default();
            let components = node.components_builder().executor(provider);
            let launcher = builder
                .with_types()
                .with_components(components)
                .with_add_ons(EthereumAddOns::default());

            let NodeHandle {
                node,
                node_exit_future,
            } = launcher.launch_with_debug_capabilities().await?;

            // Install ress subprotocol.
            if ress_args.enabled {
                install_ress_subprotocol(
                    ress_args,
                    node.provider,
                    node.block_executor,
                    node.network,
                    node.task_executor,
                    node.add_ons_handle.engine_events.new_listener(),
                )?;
            }

            node_exit_future.await
        })
    {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
