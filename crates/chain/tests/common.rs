use alloy_consensus::SignableTransaction;
use alloy_consensus::{TxLegacy, constants::ETH_TO_WEI};
use alloy_eips::eip7002::{
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS, WITHDRAWAL_REQUEST_PREDEPLOY_CODE,
};
use alloy_genesis::Genesis;
use alloy_primitives::{Address, B256, b256, hex};
use alloy_primitives::{Bytes, TxKind};
use alloy_primitives::{U256, fixed_bytes, keccak256};
use futures_util::StreamExt;
use reth::api::{EngineTypes, NodeTypes};
use reth::args::RpcServerArgs;
use reth::builder::rpc::RethRpcAddOns;
use reth::builder::{FullNode, FullNodeComponents, NodeConfig};
use reth::revm::database_interface::EmptyDB;
use reth::rpc::api::EngineEthApiClient;
use reth_chainspec::{ChainSpecBuilder, MAINNET};
use reth_ethereum::chainspec::ChainSpec;
use reth_ethereum::primitives::SignedTransaction;
use reth_ethereum::provider::CanonStateSubscriptions;
use reth_ethereum_primitives::{Block, BlockBody, Transaction};
use reth_primitives::TransactionSigned;
use reth_primitives_traits::crypto::secp256k1::sign_message;
use reth_primitives_traits::{
    Block as _, RecoveredBlock, crypto::secp256k1::public_key_to_address,
};
use revm::{
    database::CacheDB,
    state::{AccountInfo, Bytecode},
};
use secp256k1::Keypair;
use secp256k1::Secp256k1;
use std::error::Error;
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

pub fn get_random_keypair() -> (Keypair, Address) {
    let secp = Secp256k1::new();
    let sender_key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    let sender_address = public_key_to_address(sender_key_pair.public_key());

    (sender_key_pair, sender_address)
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

    let tx = sign_tx_with_key_pair(
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

pub fn sign_tx_with_key_pair(key_pair: Keypair, tx: Transaction) -> TransactionSigned {
    let signature = sign_message(
        B256::from_slice(&key_pair.secret_bytes()[..]),
        tx.signature_hash(),
    )
    .unwrap();

    TransactionSigned::new_unhashed(tx, signature)
}

pub fn get_test_node_config() -> NodeConfig<ChainSpec> {
    NodeConfig::test()
        .dev()
        .with_rpc(RpcServerArgs::default().with_http())
        .with_chain(custom_chain())
}

pub async fn send_compare_transaction<Node, AddOns, Engine>(
    node: FullNode<Node, AddOns>,
) -> Result<(), Box<dyn Error>>
where
    Engine: EngineTypes,
    Node: FullNodeComponents<Types: NodeTypes<Payload = Engine>>,
    AddOns: RethRpcAddOns<Node>,
{
    let mut notifications = node.provider.canonical_state_stream();

    // submit tx through rpc
    let raw_tx = hex!(
        "02f876820a28808477359400847735940082520894ab0840c0e43688012c1adb0f5e3fc665188f83d28a029d394a5d630544000080c080a0a044076b7e67b5deecc63f61a8d7913fab86ca365b344b5759d1fe3563b4c39ea019eab979dd000da04dfc72bb0377c092d30fd9e1cab5ae487de49586cc8b0090"
    );
    let hash = EngineEthApiClient::<B256, B256>::send_raw_transaction(
        &node.engine_http_client(),
        raw_tx.into(),
    )
    .await?;
    let expected = b256!("0xb1c6512f4fc202c04355fbda66755e0e344b152e633010e8fd75ecec09b63398");

    println!("submitted transaction: {hash}");
    println!("expected transaction: {expected}");

    notifications.next().await;
    let head = notifications.next().await.unwrap();
    println!("got notification head: {:?}", head);

    use reth_primitives_traits::BlockBody;
    let tx = &head.tip().body().transactions().iter().next().unwrap();
    println!("notification transaction {:?}", tx.tx_hash());

    println!("mined transaction: {hash}");

    Ok(())
}

pub fn custom_chain() -> Arc<ChainSpec> {
    let custom_genesis = r#"
{
    "nonce": "0x42",
    "timestamp": "0x0",
    "extraData": "0x5343",
    "gasLimit": "0x5208",
    "difficulty": "0x400000000",
    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "coinbase": "0x0000000000000000000000000000000000000000",
    "alloc": {
        "0x6Be02d1d3665660d22FF9624b7BE0551ee1Ac91b": {
            "balance": "0x4a47e3c12448f4ad000000"
        }
    },
    "number": "0x0",
    "gasUsed": "0x0",
    "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "config": {
        "ethash": {},
        "chainId": 2600,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "terminalTotalDifficulty": 0,
        "terminalTotalDifficultyPassed": true,
        "shanghaiTime": 0
    }
}
"#;
    let genesis: Genesis = serde_json::from_str(custom_genesis).unwrap();
    Arc::new(genesis.into())
}
