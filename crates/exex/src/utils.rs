use reth_primitives::{revm_primitives, Header};

pub fn convert_header(raw: &Header) -> alloy_consensus::Header {
    alloy_consensus::Header::try_from(raw.clone()).unwrap()

    // reth_primitives::Header {
    //     parent_hash: raw.parent_hash,
    //     ommers_hash: raw.ommers_hash,
    //     beneficiary: raw.beneficiary,
    //     state_root: raw.state_root,
    //     transactions_root: raw.transactions_root,
    //     receipts_root: raw.receipts_root,
    //     withdrawals_root: raw.withdrawals_root,
    //     logs_bloom: raw.logs_bloom,
    //     difficulty: raw.difficulty,
    //     number: raw.number,
    //     gas_limit: raw.gas_limit,
    //     gas_used: raw.gas_used,
    //     timestamp: raw.timestamp,
    //     extra_data: raw.extra_data.clone(),
    //     mix_hash: raw.mix_hash,
    //     nonce: raw.nonce,
    //     base_fee_per_gas: raw.base_fee_per_gas,
    //     blob_gas_used: raw.blob_gas_used,
    //     excess_blob_gas: raw.excess_blob_gas,
    //     parent_beacon_block_root: raw.parent_beacon_block_root,
    //     requests_root: raw.requests_hash,
    // }
}

pub fn try_copy_tx_env(raw: reth_revm::primitives::TxEnv) -> revm_primitives::types::TransactionEnv {}