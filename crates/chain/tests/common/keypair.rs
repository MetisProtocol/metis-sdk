use alloy_consensus::SignableTransaction;
use alloy_primitives::{Address, B256};
use reth_ethereum_primitives::Transaction;
use reth_primitives::TransactionSigned;
use reth_primitives_traits::crypto::secp256k1::{public_key_to_address, sign_message};
use secp256k1::Keypair;
use secp256k1::Secp256k1;

pub fn get_random_keypair() -> (Keypair, Address) {
    let secp = Secp256k1::new();
    let sender_key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    let sender_address = public_key_to_address(sender_key_pair.public_key());

    (sender_key_pair, sender_address)
}

pub fn sign_tx_with_key_pair(key_pair: Keypair, tx: Transaction) -> TransactionSigned {
    let signature = sign_message(
        B256::from_slice(&key_pair.secret_bytes()[..]),
        tx.signature_hash(),
    )
    .unwrap();

    TransactionSigned::new_unhashed(tx, signature)
}
