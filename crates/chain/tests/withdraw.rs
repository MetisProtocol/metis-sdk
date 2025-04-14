use alloy_evm::block::BlockExecutionResult;
use metis_chain::provider::BlockParallelExecutorProvider;
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_evm_ethereum::EthEvmConfig;
use std::error::Error;

pub mod common;
// #[path = "../../pe/tests/common/mod.rs"]
// pub mod common;

#[tokio::test]
async fn test_withdraw() -> Result<(), Box<dyn Error>> {
    let (keypair, sender) = common::get_random_keypair();
    let (chain_spec, db, recovered_block) = common::get_test_withdraw_config(sender, keypair);
    let config = EthEvmConfig::new(chain_spec);
    let provider = BlockParallelExecutorProvider::new(config);
    let mut executor = provider.executor(db);

    let BlockExecutionResult {
        receipts, requests, ..
    } = executor.execute_one(recovered_block).unwrap();

    let receipt = receipts.first().unwrap();

    println!("receipt {:?}", receipt);
    assert!(receipt.success);

    // There should be exactly one entry with withdrawal requests
    println!("request {:?}", requests);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0][0], 1);

    Ok(())
}
