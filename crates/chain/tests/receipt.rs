use alloy_evm::block::BlockExecutionResult;
use metis_chain::provider::BlockParallelExecutorProvider;
use pretty_assertions::assert_eq;
use reth_evm::execute::{BasicBlockExecutorProvider, BlockExecutorProvider, Executor};
use reth_evm_ethereum::EthEvmConfig;
use std::error::Error;

pub mod common;

#[tokio::test]
async fn test_compare_receipt() -> Result<(), Box<dyn Error>> {
    let (keypair, sender) = common::get_random_keypair();
    println!("Sending tx: {:?}", sender);

    let parallel_receipt = {
        let (chain_spec, db, recovered_block) = common::get_test_withdraw_config(sender, keypair);
        let config = EthEvmConfig::new(chain_spec);
        let provider = BlockParallelExecutorProvider::new(config);
        let mut executor = provider.executor(db);

        let BlockExecutionResult { receipts, .. } = executor.execute_one(&recovered_block).unwrap();
        receipts.first().unwrap().clone()
    };

    let custom_receipt = {
        let (chain_spec, db, recovered_block) = common::get_test_withdraw_config(sender, keypair);
        let provider = BasicBlockExecutorProvider::new(EthEvmConfig::new(chain_spec));
        let mut executor = provider.executor(db);
        let BlockExecutionResult { receipts, .. } = executor.execute_one(&recovered_block).unwrap();
        receipts.first().unwrap().clone()
    };

    assert_eq!(parallel_receipt, custom_receipt);

    Ok(())
}
