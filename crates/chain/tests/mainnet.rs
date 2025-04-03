#[path = "../../pe/tests/common/mod.rs"]
mod common;

use reth_chainspec::{ChainSpec, MAINNET};
use reth_evm::execute::BlockExecutorProvider;
use reth_evm_ethereum::EthEvmConfig;
use reth::revm::State;
use metis_chain::{
    provider::BlockParallelExecutorProvider,
    state::StateStorageAdapter,
};

// TODO(FK): test provider with pevm/data/blocks
#[test]
fn mainnet_blocks_from_disk() {

    let chain_spec = MAINNET.clone();
    let eth_evm_config = EthEvmConfig::new(chain_spec);
    let provider = BlockParallelExecutorProvider::new(eth_evm_config);
    let state_db = State::builder().with_database(db).with_bundle_update().without_state_clear().build();
    let state_db_adapter = StateStorageAdapter::new(state_db);
    let executor = provider.executor(state_db_adapter);
    // common::for_each_block_from_disk(|block, storage| {
    //     // Run several times to try catching a race condition if there is any.
    //     // 1000~2000 is a better choice for local testing after major changes.
    //     for _ in 0..3 {
    //         common::test_execute_alloy(&Ethereum::mainnet(), &storage, block.clone(), true)
    //     }
    // });
}