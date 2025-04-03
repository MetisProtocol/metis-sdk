//! Test small blocks that we have specific handling for, like implicit fine-tuning
//! the concurrency level, falling back to sequential processing, etc.

use alloy_primitives::{Address, U256};
use metis_pe::InMemoryStorage;
use metis_pe::chain::Ethereum;
use revm::context::{TransactTo, TxEnv};

pub mod common;

#[test]
fn empty_revm_block() {
    common::test_execute_revm(&Ethereum::mainnet(), InMemoryStorage::default(), Vec::new());
}

#[test]
fn one_tx_revm_block() {
    common::test_execute_revm(
        &Ethereum::mainnet(),
        InMemoryStorage::new(
            [common::mock_account(0)].into_iter().collect(),
            Default::default(),
            Default::default(),
        ),
        vec![TxEnv {
            caller: Address::ZERO,
            kind: TransactTo::Call(Address::ZERO),
            value: U256::from(1),
            gas_price: 1_u128,
            ..TxEnv::default()
        }],
    );
}
