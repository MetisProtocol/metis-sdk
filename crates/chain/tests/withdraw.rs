use alloy_consensus::{TxLegacy, constants::ETH_TO_WEI};
use alloy_eips::eip4788::{BEACON_ROOTS_ADDRESS, BEACON_ROOTS_CODE};
use alloy_eips::eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS;
use alloy_evm::block::BlockExecutionResult;
use alloy_primitives::b256;
use alloy_primitives::{Bytes, TxKind};
use alloy_primitives::{U256, fixed_bytes, keccak256};
use metis_chain::provider::BlockParallelExecutorProvider;
use reth::revm::database_interface::EmptyDB;
use reth_chainspec::{ChainSpecBuilder, MAINNET};
use reth_ethereum_primitives::{Block, BlockBody, Transaction};
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives_traits::{Block as _, crypto::secp256k1::public_key_to_address};
use revm::{
    database::CacheDB,
    state::{AccountInfo, Bytecode},
};
use secp256k1::{Keypair, Secp256k1};
use std::error::Error;
use std::sync::Arc;

pub mod common;
// #[path = "../../pe/tests/common/mod.rs"]
// pub mod common;

#[tokio::test]
async fn test_withdraw() -> Result<(), Box<dyn Error>> {
    // crate eip 7002 chain spec
    let chain_spec = Arc::new(
        ChainSpecBuilder::from(&*MAINNET)
            .shanghai_activated()
            .cancun_activated()
            .prague_activated()
            .build(),
    );

    // crate database
    let mut db = CacheDB::new(EmptyDB::default());
    let beacon_root_contract_account = AccountInfo {
        balance: U256::ZERO,
        code_hash: keccak256(BEACON_ROOTS_CODE.clone()),
        nonce: 1,
        code: Some(Bytecode::new_raw(BEACON_ROOTS_CODE.clone())),
    };
    db.insert_account_info(BEACON_ROOTS_ADDRESS, beacon_root_contract_account);

    // generate user address
    let secp = Secp256k1::new();
    let sender_key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    let sender_address = public_key_to_address(sender_key_pair.public_key());

    db.insert_account_info(
        sender_address,
        AccountInfo {
            nonce: 1,
            balance: U256::from(ETH_TO_WEI),
            ..Default::default()
        },
    );

    // https://github.com/lightclient/sys-asm/blob/9282bdb9fd64e024e27f60f507486ffb2183cba2/test/Withdrawal.t.sol.in#L36
    let validator_public_key = fixed_bytes!(
        "111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111"
    );
    let withdrawal_amount = fixed_bytes!("0203040506070809");
    let input: Bytes = [&validator_public_key[..], &withdrawal_amount[..]]
        .concat()
        .into();
    assert_eq!(input.len(), 56);

    let mut header = chain_spec.genesis_header().clone();
    header.gas_limit = 1_500_000;
    // measured
    header.gas_used = 135_856;
    header.receipts_root =
        b256!("0xb31a3e47b902e9211c4d349af4e4c5604ce388471e79ca008907ae4616bb0ed3");

    let tx = common::sign_tx_with_key_pair(
        sender_key_pair,
        Transaction::Legacy(TxLegacy {
            chain_id: Some(chain_spec.chain.id()),
            nonce: 1,
            gas_price: header.base_fee_per_gas.unwrap().into(),
            gas_limit: header.gas_used,
            to: TxKind::Call(WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS),
            // `MIN_WITHDRAWAL_REQUEST_FEE`
            value: U256::from(2),
            input,
        }),
    );

    let config = EthEvmConfig::new(chain_spec);
    let provider = BlockParallelExecutorProvider::new(config);
    let mut executor = provider.executor(db);

    let recovered_block = &Block {
        header,
        body: BlockBody {
            transactions: vec![tx],
            ..Default::default()
        },
    }
    .try_into_recovered()
    .unwrap();

    let BlockExecutionResult {
        receipts, requests, ..
    } = executor.execute_one(&recovered_block).unwrap();

    let receipt = receipts.first().unwrap();
    println!("receipt {:?}", receipt);
    assert!(receipt.success);

    // There should be exactly one entry with withdrawal requests
    println!("request {:?}", requests);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0][0], 1);

    Ok(())
}
