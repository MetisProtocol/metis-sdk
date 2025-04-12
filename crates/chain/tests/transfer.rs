use metis_chain::provider::BlockParallelExecutorProvider;
use metis_pe::InMemoryDB;
use reth_chain_state::test_utils::TestBlockBuilder;
use reth_chainspec::MAINNET;
use reth_evm::execute::BlockExecutorProvider;
use reth_evm_ethereum::EthEvmConfig;
use revm::database::WrapDatabaseRef;
use std::error::Error;

#[path = "../../pe/tests/common/mod.rs"]
pub mod common;

#[tokio::test]
async fn test_single_transfer() -> Result<(), Box<dyn Error>> {
    let chain_spec = MAINNET.clone();

    let mut test_block_builder = TestBlockBuilder::eth().with_chain_spec((*chain_spec).clone());

    let blocks: Vec<_> = test_block_builder.get_executed_blocks(0..5).collect();

    let recovered_block = test_block_builder
        .generate_random_block(6, blocks.last().unwrap().recovered_block().hash());

    let memory_db = InMemoryDB::new(
        (0..=6).map(common::mock_account).collect(),
        Default::default(),
        Default::default(),
    );
    let wrap_db = WrapDatabaseRef::from(memory_db);
    let config = EthEvmConfig::new(chain_spec);
    let provider = BlockParallelExecutorProvider::new(config);
    let mut executor = provider.executor(wrap_db);

    executor
        .execute_block(&recovered_block)
        .expect("failed to execute block");

    Ok(())
}
