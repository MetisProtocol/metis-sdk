use alloy_consensus::{TxLegacy, constants::ETH_TO_WEI};
use alloy_eips::eip7002::{
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_CODE,
};
use alloy_primitives::{Address, b256};
use alloy_primitives::{Bytes, TxKind};
use alloy_primitives::{U256, fixed_bytes, keccak256};
use reth::revm::database_interface::EmptyDB;
use reth_chainspec::{ChainSpecBuilder, MAINNET};
use reth_ethereum::chainspec::ChainSpec;
use reth_ethereum_primitives::{Block, BlockBody, Transaction};
use reth_primitives_traits::{Block as _, RecoveredBlock};
use revm::{
    database::CacheDB,
    state::{AccountInfo, Bytecode},
};
use secp256k1::Keypair;
use std::sync::Arc;

pub fn get_test_withdraw_config(
    sender: Address,
    keypair: Keypair,
) -> (Arc<ChainSpec>, CacheDB<EmptyDB>, RecoveredBlock<Block>) {
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
    let withdrawal_requests_contract_account = AccountInfo {
        balance: U256::ZERO,
        code_hash: keccak256(WITHDRAWAL_REQUEST_PREDEPLOY_CODE.clone()),
        nonce: 1,
        code: Some(Bytecode::new_raw(WITHDRAWAL_REQUEST_PREDEPLOY_CODE.clone())),
    };
    db.insert_account_info(
        WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
        withdrawal_requests_contract_account,
    );

    db.insert_account_info(
        sender,
        AccountInfo {
            nonce: 1,
            balance: U256::from(ETH_TO_WEI),
            ..Default::default()
        },
    );

    let block = get_test_block_with_single_withdraw_tx(&chain_spec, &keypair);
    (chain_spec, db, block)
}

pub fn get_test_block_with_single_withdraw_tx(
    chain_spec: &ChainSpec,
    sender_key_pair: &Keypair,
) -> RecoveredBlock<Block> {
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

    let tx = crate::common::keypair::sign_tx_with_key_pair(
        *sender_key_pair,
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

    Block {
        header,
        body: BlockBody {
            transactions: vec![tx],
            ..Default::default()
        },
    }
    .try_into_recovered()
    .unwrap()
}
