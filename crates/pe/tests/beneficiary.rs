//! Tests for the beneficiary account, especially for the lazy update of its balance to avoid
//! "implicit" dependency among consecutive transactions.

use metis_pe::InMemoryStorage;
use metis_pe::chain::Ethereum;
use rand::random;
use revm::context::{TransactTo, TxEnv};
use revm::primitives::{Address, U256, alloy_primitives::U160};

pub mod common;

const BLOCK_SIZE: usize = 100_000;

fn test_beneficiary(get_address: fn(usize) -> Address) {
    common::test_execute_revm(
        &Ethereum::mainnet(),
        // Mock the beneficiary account (`Address:ZERO`) and the next `BLOCK_SIZE` user accounts.
        InMemoryStorage::new(
            (0..=BLOCK_SIZE).map(common::mock_account).collect(),
            Default::default(),
            Default::default(),
        ),
        // Mock `BLOCK_SIZE` transactions sending some tokens to itself.
        // Skipping `Address::ZERO` as the beneficiary account.
        (1..=BLOCK_SIZE)
            .map(|i| {
                // Randomly insert a beneficiary spending every ~256 txs
                let address = get_address(i);
                TxEnv {
                    caller: address,
                    kind: TransactTo::Call(address),
                    value: U256::from(1),
                    gas_price: 1_u128,
                    ..TxEnv::default()
                }
            })
            .collect(),
    );
}

#[test]
fn beneficiary_random() {
    test_beneficiary(|i| {
        // Randomly insert a beneficiary spending every ~256 txs
        if random::<u8>() == 0 {
            Address::from(U160::from(0))
        } else {
            Address::from(U160::from(i))
        }
    });
}

#[test]
fn beneficiary_heavy_evaluation() {
    test_beneficiary(|i| {
        // Setting only the last tx as beneficiary for a heavy
        // evaluation all the way to the top of the block.
        if i == BLOCK_SIZE {
            Address::from(U160::from(0))
        } else {
            Address::from(U160::from(i))
        }
    });
}
