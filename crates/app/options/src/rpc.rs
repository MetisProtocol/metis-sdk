use clap::{Args, Subcommand, ValueEnum};
use metis_primitives::{Address, Bytes, U256};
use std::path::PathBuf;
use tendermint_rpc::Url;

#[derive(Args, Debug)]
pub struct RpcArgs {
    /// The URL of the Tendermint node's RPC endpoint.
    #[arg(
        long,
        short,
        default_value = "http://127.0.0.1:26657",
        env = "TENDERMINT_RPC_URL"
    )]
    pub url: Url,

    /// An optional HTTP/S proxy through which to submit requests to the
    /// Tendermint node's RPC endpoint.
    #[arg(long)]
    pub proxy_url: Option<Url>,

    #[command(subcommand)]
    pub command: RpcCommands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum RpcCommands {
    /// Send an ABCI query.
    Query {
        /// Block height to query; 0 means latest.
        #[arg(long, short = 'b', default_value_t = 0)]
        height: u64,
        #[command(subcommand)]
        command: RpcQueryCommands,
    },
    /// Transfer tokens between accounts.
    Transfer {
        /// Address of the actor to send the message to.
        #[arg(long, short)]
        to: Address,
        #[command(flatten)]
        args: TransArgs,
    },
    /// Subcommands related to EVM.
    Evm {
        #[command(subcommand)]
        command: RpcEvmCommands,
        #[command(flatten)]
        args: TransArgs,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum RpcQueryCommands {
    /// Get the state of an actor; print it as JSON.
    ActorState {
        /// Address of the actor to query.
        #[arg(long, short)]
        address: Address,
    },
    /// Get the slowly changing state parameters.
    StateParams,
}

#[derive(Subcommand, Debug, Clone)]
pub enum RpcEvmCommands {
    /// Deploy an EVM contract from source; print the results as JSON.
    Create {
        /// Path to a compiled Solidity contract, expected to be in hexadecimal format.
        #[arg(long, short)]
        contract: PathBuf,
        /// ABI encoded constructor arguments passed to the EVM, expected to be in hexadecimal format.
        #[arg(long, short, default_value = "")]
        constructor_args: Bytes,
    },
    /// Invoke an EVM contract; print the results as JSON with the return data rendered in hexadecimal format.
    Invoke {
        #[command(flatten)]
        args: EvmArgs,
    },
    /// Call an EVM contract without a transaction; print the results as JSON with the return data rendered in hexadecimal format.
    Call {
        #[command(flatten)]
        args: EvmArgs,
        /// Block height to query; 0 means latest.
        #[arg(long, short = 'b', default_value_t = 0)]
        height: u64,
    },
    /// Estimate the gas required to execute a FEVM invocation.
    EstimateGas {
        #[command(flatten)]
        args: EvmArgs,
        /// Block height to query; 0 means latest.
        #[arg(long, short = 'b', default_value_t = 0)]
        height: u64,
    },
}

// todo call evm contract args
/// Arguments common to FEVM method calls.
#[derive(Args, Debug, Clone)]
pub struct EvmArgs {
    /// Either the actor ID based or the EAM delegated address of the contract to call.
    #[arg(long, short)]
    pub contract: Address,
    // /// ABI encoded method hash, expected to be in hexadecimal format.
    // #[arg(long, short)]
    // pub method: Bytes,
    // /// ABI encoded call arguments passed to the EVM, expected to be in hexadecimal format.
    // #[arg(long, short, default_value = "")]
    // pub method_args: Bytes,
}

/// Arguments common to transactions and transfers.
#[derive(Args, Debug, Clone)]
pub struct TransArgs {
    /// Name of chain the for which the message will be signed.
    #[arg(long, short, env = "FM_CHAIN_NAME")]
    pub chain_name: String,
    /// Amount of tokens to send, in full FIL, not atto.
    #[arg(long, short, default_value = "0")]
    pub value: U256,
    /// Path to the secret key of the sender to sign the transaction.
    #[arg(long, short)]
    pub secret_key: PathBuf,
    /// Sender account nonce.
    #[arg(long, short = 'n')]
    pub sequence: u64,
    /// Maximum amount of gas that can be charged.
    #[arg(long, default_value_t = 10_000_000_000)] // Default from ref-fvm testkit.
    pub gas_limit: u64,
    /// Price of gas.
    ///
    /// Any discrepancy between this and the base fee is paid for
    /// by the validator who puts the transaction into the block.
    #[arg(long, default_value = "0")]
    pub gas_fee_cap: U256,
    /// Gas premium.
    #[arg(long, default_value = "0")]
    pub gas_premium: U256,
    /// Whether to wait for the results from Tendermint or not.
    #[arg(long, short, default_value = "commit")]
    pub broadcast_mode: BroadcastMode,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum BroadcastMode {
    /// Do no wait for the results.
    Async,
    /// Wait for the result of `check_tx`.
    Sync,
    /// Wait for the result of `deliver_tx`.
    Commit,
}
