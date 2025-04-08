use reth_primitives::TxType;

pub(crate) fn from_receipt(src: alloy_rpc_types_eth::Receipt) -> reth::primitives::Receipt {
    reth::primitives::Receipt {
        // todo(fk): how to get the receipt tx_type
        tx_type: TxType::Legacy,
        success: src.status.coerce_status(),
        cumulative_gas_used: src.cumulative_gas_used,
        logs: src.logs,
    }
}
