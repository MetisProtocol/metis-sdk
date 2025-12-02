#![allow(missing_docs)]

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();
use clap::Parser;
use metis_chain::hook_provider::HookExecutorBuilder;
use metis_chain::op_provider::OpParallelNode;
use metis_chain::provider::ParallelExecutorBuilder;
use reth::cli::Cli;
use reth_node_ethereum::EthereumNode;
use reth_node_ethereum::node::EthereumAddOns;
use reth_optimism_cli::{Cli as OpCli, chainspec::OpChainSpecParser};
use reth_optimism_node::{OpNode, args::RollupArgs};
use tracing::info;

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    if std::env::var_os("ENABLE_OP_EXECUTOR").is_some() {
        if let Err(err) = OpCli::<OpChainSpecParser, RollupArgs>::parse().run(
            async move |builder, rollup_args| {
                info!(target: "reth::cli", "Launching node");
                if std::env::var_os("ENABLE_PARALLEL_EXECUTOR").is_some() {
                    let handle = builder.node(OpParallelNode::new(OpNode::new(rollup_args)));
                    handle.launch().await?.wait_for_node_exit().await
                } else {
                    let handle = builder.node(OpNode::new(rollup_args));
                    // handle.launch().await?.wait_for_node_exit().await
                    handle
                        .launch_with_debug_capabilities()
                        .await?
                        .wait_for_node_exit()
                        .await
                }
            },
        ) {
            eprintln!("Error: {err:?}");
            std::process::exit(1);
        }
    } else {
        if let Err(err) = Cli::parse_args().run(async move |builder, _| {
            info!(target: "metis::cli", "Launching node");
            if std::env::var_os("ENABLE_PARALLEL_EXECUTOR").is_some() {
                let handle = builder
                    // Use the default ethereum node types
                    .with_types::<EthereumNode>()
                    // Configure the components of the node
                    // use default ethereum components but use our parallel executor.
                    .with_components(
                        EthereumNode::components().executor(ParallelExecutorBuilder::default()),
                    )
                    .with_add_ons(EthereumAddOns::default());
                handle.launch().await?.wait_for_node_exit().await
            } else {
                let handle = builder
                    // Use the default ethereum node types
                    .with_types::<EthereumNode>()
                    // Configure the components of the node
                    // use default ethereum components but use our parallel executor.
                    .with_components(
                        EthereumNode::components().executor(HookExecutorBuilder::default()),
                    )
                    .with_add_ons(EthereumAddOns::default());
                // handle.launch().await?.wait_for_node_exit().await
                handle
                    .launch_with_debug_capabilities()
                    .await?
                    .wait_for_node_exit()
                    .await
            }
        }) {
            eprintln!("Error: {err:?}");
            std::process::exit(1);
        }
    }
}
