//! Test raw transfers -- only send some ETH from one account to another without extra data.

use metis_pe::{InMemoryStorage, chain::Ethereum};
use rand::random;
use revm::context::{TransactTo, TxEnv};
use revm::primitives::{Address, U256, alloy_primitives::U160};

pub mod common;

#[test]
fn raw_transfers_independent() {
    let block_size = 100_000; // number of transactions
    common::test_execute_revm(
        &Ethereum::mainnet(),
        // Mock the beneficiary account (`Address:ZERO`) and the next `block_size` user accounts.
        InMemoryStorage::new(
            (0..=block_size).map(common::mock_account).collect(),
            Default::default(),
            Default::default(),
        ),
        // Mock `block_size` transactions sending some tokens to itself.
        // Skipping `Address::ZERO` as the beneficiary account.
        (1..=block_size)
            .map(|i| {
                let address = Address::from(U160::from(i));
                TxEnv {
                    caller: address,
                    kind: TransactTo::Call(address),
                    value: U256::from(1),
                    gas_limit: common::RAW_TRANSFER_GAS_LIMIT,
                    gas_price: 1_u128,
                    ..TxEnv::default()
                }
            })
            .collect(),
    );
}

// The same sender sending multiple transfers with increasing nonces.
// These must be detected and executed in the correct order.
#[test]
fn raw_transfers_same_sender_multiple_txs() {
    let block_size = 5_000; // number of transactions

    let same_sender_address = Address::from(U160::from(1));
    let mut same_sender_nonce: u64 = 0;

    common::test_execute_revm(
        &Ethereum::mainnet(),
        // Mock the beneficiary account (`Address:ZERO`) and the next `block_size` user accounts.
        InMemoryStorage::new(
            (0..=block_size).map(common::mock_account).collect(),
            Default::default(),
            Default::default(),
        ),
        (1..=block_size)
            .map(|i| {
                // Insert a "parallel" transaction every ~256 transactions
                // after the first ~30 guaranteed from the same sender.
                let (address, nonce) = if i > 30 && random::<u8>() == 0 {
                    (Address::from(U160::from(i)), 1)
                } else {
                    same_sender_nonce += 1;
                    (same_sender_address, same_sender_nonce)
                };
                TxEnv {
                    caller: address,
                    kind: TransactTo::Call(address),
                    value: U256::from(1),
                    gas_limit: common::RAW_TRANSFER_GAS_LIMIT,
                    gas_price: 1_u128,
                    nonce,
                    ..TxEnv::default()
                }
            })
            .collect(),
    );
}
