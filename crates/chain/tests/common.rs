use alloy_consensus::SignableTransaction;
use alloy_genesis::Genesis;
use alloy_primitives::{B256, b256, hex};
use futures_util::StreamExt;
pub use rand::Rng;
use reth::api::{EngineTypes, NodeTypes};
use reth::args::RpcServerArgs;
use reth::builder::rpc::RethRpcAddOns;
use reth::builder::{BlockBody, FullNode, FullNodeComponents, NodeConfig};
use reth::rpc::api::EngineEthApiClient;
use reth_ethereum::chainspec::ChainSpec;
use reth_ethereum::primitives::SignedTransaction;
use reth_ethereum::provider::CanonStateSubscriptions;
use reth_primitives::{Transaction, TransactionSigned};
use reth_primitives_traits::crypto::secp256k1::sign_message;
use secp256k1::Keypair;
use std::error::Error;
use std::sync::Arc;

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
    //let tx = &head.tip().body().transactions().next().unwrap();
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
