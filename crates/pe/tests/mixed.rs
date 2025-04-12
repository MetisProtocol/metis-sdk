//! Test raw transfers -- A block with random raw transfers, ERC-20 transfers, and Uniswap swaps.

use metis_pe::{AccountState, Bytecodes, EvmAccount, InMemoryDB};
use rand::random;
use revm::context::{TransactTo, TxEnv};
use revm::primitives::{Address, U256};
use std::collections::HashMap;
use std::sync::Arc;

pub mod common;
pub mod erc20;
pub mod uniswap;

#[test]
fn mixed_block() {
    let target_block_size = 100_000; // number of transactions
    let mut block_size = 0;
    let mut final_state = AccountState::default();
    final_state.insert(Address::ZERO, EvmAccount::default()); // Beneficiary
    let mut final_bytecodes = Bytecodes::default();
    let mut final_txs = Vec::new();
    // 1 to 10
    let small_random = || (random::<u8>() % 10 + 1) as usize;
    let mut nonce_nums = HashMap::new();
    while block_size < target_block_size {
        match small_random() % 3 {
            0 => {
                // Raw transfers are more popular
                let no_txs = random::<u16>();
                for _ in 0..no_txs {
                    let (address, account) = common::mock_account(small_random());
                    let nonce = nonce_nums.entry(address).or_insert(0);
                    *nonce += 1;
                    final_state.insert(address, account);
                    final_txs.push(TxEnv {
                        caller: address,
                        kind: TransactTo::Call(address), // TODO: Randomize for tighter test
                        value: U256::from(1),
                        gas_limit: common::RAW_TRANSFER_GAS_LIMIT,
                        gas_price: 1_u128,
                        nonce: *nonce,
                        ..TxEnv::default()
                    });
                }
                block_size += no_txs as usize;
            }
            1 => {
                let (state, bytecodes, txs) =
                    erc20::generate_cluster(small_random(), small_random(), small_random());
                block_size += txs.len();
                final_state.extend(state);
                final_bytecodes.extend(bytecodes);
                final_txs.extend(txs);
            }
            _ => {
                let (state, bytecodes, txs) =
                    uniswap::generate_cluster(small_random(), small_random());
                block_size += txs.len();
                final_state.extend(state);
                final_bytecodes.extend(bytecodes);
                final_txs.extend(txs);
            }
        }
    }
    common::test_execute_revm(
        InMemoryDB::new(final_state, Arc::new(final_bytecodes), Default::default()),
        // TODO: Shuffle transactions to scatter dependencies around the block.
        // Note that we'll need to guarantee that the nonces are increasing.
        final_txs,
    );
}
