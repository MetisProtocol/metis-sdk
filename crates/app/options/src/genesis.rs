use std::path::PathBuf;

use clap::{Args, Subcommand};
use metis_primitives::U256;

#[derive(Subcommand, Debug)]
pub enum GenesisCommands {
    /// Create a new Genesis file, with accounts and validators to be added later.
    New(GenesisNewArgs),
    /// Add an account to the genesis file.
    AddAccount(GenesisAddAccountArgs),
    /// Add a multi-sig account to the genesis file.
    AddMultisig(GenesisAddMultisigArgs),
    /// Add a validator to the genesis file.
    AddValidator(GenesisAddValidatorArgs),
    /// Convert the genesis file into the format expected by Tendermint.
    IntoTendermint(GenesisIntoTendermintArgs),
}

#[derive(Args, Debug)]
pub struct GenesisArgs {
    /// Path to the genesis JSON file.
    #[arg(long, short)]
    pub genesis_file: PathBuf,

    #[command(subcommand)]
    pub command: GenesisCommands,
}

#[derive(Args, Debug)]
pub struct GenesisNewArgs {
    /// Genesis timestamp as seconds since Unix epoch.
    #[arg(long, short)]
    pub timestamp: u64,
    /// Name of the network and chain.
    #[arg(long, short = 'n')]
    pub chain_name: String,
    /// Network version, governs which set of built-in actors to use.
    #[arg(long, short = 'v', default_value = "18")]
    pub network_version: String,
    /// Base fee for running transactions in atto.
    #[arg(long, short = 'f')]
    pub base_fee: U256,
}

#[derive(Args, Debug)]
pub struct GenesisAddAccountArgs {
    /// Path to the Secp256k1 public key exported in base64 format.
    #[arg(long, short)]
    pub public_key: PathBuf,
    /// Initial balance in full FIL units.
    #[arg(long, short)]
    pub balance: U256,
}

#[derive(Args, Debug)]
pub struct GenesisAddMultisigArgs {
    /// Path to the Secp256k1 public key exported in base64 format, one for each signatory.
    #[arg(long, short)]
    pub public_key: Vec<PathBuf>,
    /// Initial balance in full FIL units.
    #[arg(long, short)]
    pub balance: U256,
    /// Number of signatures required.
    #[arg(long, short)]
    pub threshold: u64,
    /// Linear unlock duration in block heights.
    #[arg(long, short = 'd')]
    pub vesting_duration: u64,
    /// Linear unlock start block height.
    #[arg(long, short = 's')]
    pub vesting_start: u64,
}

#[derive(Args, Debug)]
pub struct GenesisAddValidatorArgs {
    /// Path to the Secp256k1 public key exported in base64 format.
    #[arg(long, short)]
    pub public_key: PathBuf,
    /// Voting power.
    #[arg(long, short = 'v')]
    pub power: u64,
}

#[derive(Args, Debug)]
pub struct GenesisIntoTendermintArgs {
    /// Output file name for the Tendermint genesis JSON file.
    #[arg(long, short)]
    pub out: PathBuf,
    /// Maximum block size in bytes.
    #[arg(long, default_value_t = 22020096)]
    pub block_max_bytes: u64,
}
