//! Module common for all tests

use std::{
    fs::{self, File},
    io::BufReader,
    sync::Arc,
};

use alloy_primitives::Address;
use alloy_rpc_types_eth::Block;
use flate2::bufread::GzDecoder;
use hashbrown::HashMap;
use metis_pe::InMemoryStorage;
use metis_primitives::{BlockHashes, BuildSuffixHasher, EvmAccount};

/// runner module
pub mod runner;

/// runner module imports
pub use runner::{mock_account, test_execute_revm};

/// storage module
pub mod storage;

/// The gas limit for a basic transfer transaction.
pub const RAW_TRANSFER_GAS_LIMIT: u64 = 21_000;

// TODO: Put somewhere better?
/// Iterates over blocks stored on disk and processes each block using the provided handler.
pub fn for_each_block_from_disk(mut handler: impl FnMut(Block, InMemoryStorage)) {
    let data_dir = std::path::PathBuf::from("../../data");

    // TODO: Deduplicate logic with [bin/fetch.rs] when there is more usage
    let bytecodes = bincode::deserialize_from(GzDecoder::new(BufReader::new(
        File::open(data_dir.join("bytecodes.bincode.gz")).unwrap(),
    )))
    .map(Arc::new)
    .unwrap();

    let block_hashes = bincode::deserialize_from::<_, BlockHashes>(BufReader::new(
        File::open(data_dir.join("block_hashes.bincode")).unwrap(),
    ))
    .map(Arc::new)
    .unwrap();

    for block_path in fs::read_dir(data_dir.join("blocks")).unwrap() {
        let block_dir = block_path.unwrap().path();

        // Parse block
        let block = serde_json::from_reader(BufReader::new(
            File::open(block_dir.join("block.json")).unwrap(),
        ))
        .unwrap();

        // Parse state
        let accounts: HashMap<Address, EvmAccount, BuildSuffixHasher> = serde_json::from_reader(
            BufReader::new(File::open(block_dir.join("pre_state.json")).unwrap()),
        )
        .unwrap();

        handler(
            block,
            InMemoryStorage::new(accounts, Arc::clone(&bytecodes), Arc::clone(&block_hashes)),
        );
    }
}
