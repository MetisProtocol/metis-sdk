use reth::primitives::NodePrimitives;

pub fn convert_to_alloy_header<N>(header: N::BlockHeader) -> alloy_rpc_types_eth::Header
where
    N: NodePrimitives<BlockHeader = reth_primitives::Header>,
{
    let inner = alloy_consensus::Header {
        parent_hash: header.parent_hash,
        ommers_hash: header.ommers_hash,
        beneficiary: header.beneficiary,
        state_root: header.state_root,
        transactions_root: header.transactions_root,
        receipts_root: header.receipts_root,
        logs_bloom: header.logs_bloom,
        difficulty: header.difficulty,
        number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        extra_data: header.extra_data.clone(),
        mix_hash: header.mix_hash,
        nonce: header.nonce,
        base_fee_per_gas: header.base_fee_per_gas,
        withdrawals_root: header.withdrawals_root,
        blob_gas_used: header.blob_gas_used,
        excess_blob_gas: header.excess_blob_gas,
        parent_beacon_block_root: header.parent_beacon_block_root,
        requests_hash: header.requests_hash,
    };

    alloy_rpc_types_eth::Header::new(inner)
}
