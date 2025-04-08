use alloy_consensus::TxReceipt;

pub(crate) fn from_receipt(
    tx_type: alloy_consensus::TxType,
    src: alloy_rpc_types_eth::Receipt,
) -> reth::primitives::Receipt {
    let value = u8::from(tx_type);
    reth::primitives::Receipt {
        tx_type: reth::primitives::TxType::try_from(value).unwrap(),
        success: src.status(),
        cumulative_gas_used: src.cumulative_gas_used,
        logs: src.logs,
    }
}
