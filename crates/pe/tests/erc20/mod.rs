//! ERC-20 testing module

/// This module provides ERC-20 contract functionality.
pub mod contract;

use contract::ERC20Token;
use metis_pe::{Account, AccountInfo, AccountState, Bytecodes};
use rand::Rng;
use revm::context::{TransactTo, TxEnv};
use revm::primitives::{Address, U256, uint};

/// The maximum amount of gas that can be used for a transaction in this configuration.
pub const GAS_LIMIT: u64 = 35_000;

/// An estimated amount of gas that is expected to be consumed by typical transactions.
pub const ESTIMATED_GAS_USED: u64 = 29_738;

/// Sometimes we want duplicates to test
/// dependent transactions, sometimes we want to guarantee non-duplicates
/// for independent benchmarks.
fn generate_addresses(length: usize) -> Vec<Address> {
    (0..length).map(|_| Address::new(rand::random())).collect()
}

/// Generates a cluster of blockchain transactions for testing or simulation purposes.
pub fn generate_cluster(
    num_families: usize,
    num_people_per_family: usize,
    num_transfers_per_person: usize,
) -> (AccountState, Bytecodes, Vec<TxEnv>) {
    let families: Vec<Vec<Address>> = (0..num_families)
        .map(|_| generate_addresses(num_people_per_family))
        .collect();

    let people_addresses: Vec<Address> = families.clone().into_iter().flatten().collect();

    let gld_address = Address::new(rand::random());

    let gld_account = ERC20Token::new("Gold Token", "GLD", 18, 222_222_000_000_000_000_000_000u128)
        .add_balances(&people_addresses, uint!(1_000_000_000_000_000_000_U256))
        .build();

    let mut state = AccountState::from_iter([(gld_address, gld_account)]);
    let mut txs = Vec::new();

    for person in &people_addresses {
        state.insert(
            *person,
            Account {
                info: AccountInfo {
                    balance: uint!(4_567_000_000_000_000_000_000_U256),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }

    for nonce in 0..num_transfers_per_person {
        for family in &families {
            for person in family {
                let mut rng = rand::rng();
                let recipient = family[rng.random_range(0..family.len())];
                let calldata = ERC20Token::transfer(recipient, U256::from(rand::random::<u8>()));

                txs.push(TxEnv {
                    caller: *person,
                    gas_limit: GAS_LIMIT,
                    gas_price: u128::from(0xb2d05e07u64),
                    kind: TransactTo::Call(gld_address),
                    data: calldata,
                    nonce: nonce as u64,
                    ..TxEnv::default()
                })
            }
        }
    }

    let mut bytecodes = Bytecodes::default();
    for account in state.values_mut() {
        let code = account.info.code.take();
        if let Some(code) = code {
            bytecodes.insert(account.info.code_hash, code);
        }
    }

    (state, bytecodes, txs)
}
