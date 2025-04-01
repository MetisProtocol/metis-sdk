use reth::primitives::{NodePrimitives};
use reth::rpc::types::TransactionTrait;
use metis_primitives::{Address, AuthorizationList, U256};
use revm::primitives::TxEnv;

pub fn from_recovered_tx<N>(tx: N::SignedTx, sender: Address) -> TxEnv
where
    N: NodePrimitives<SignedTx = reth_primitives::Transaction>
{
    match tx.tx_type() {
        reth_primitives::TxType::Legacy => TxEnv {
            gas_limit: tx.gas_limit(),
            gas_price: tx.gas_price().map(U256::from).unwrap_or_default(),
            gas_priority_fee: None,
            transact_to: tx.to().unwrap().into(),
            value: tx.value(),
            data: tx.input().clone(),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            access_list: Default::default(),
            blob_hashes: Default::default(),
            max_fee_per_blob_gas: Default::default(),
            authorization_list: Default::default(),
            caller: sender,
            optimism: Default::default(),
        },
        reth_primitives::TxType::Eip2930 => TxEnv {
            gas_limit: tx.gas_limit(),
            gas_price: U256::from(tx.gas_price().unwrap()),
            gas_priority_fee: None,
            transact_to: tx.to().unwrap().into(),
            value: tx.value(),
            data: tx.input().clone(),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            access_list: tx.access_list().unwrap().to_vec(),
            blob_hashes: Default::default(),
            max_fee_per_blob_gas: Default::default(),
            authorization_list: Default::default(),
            caller: sender,
            optimism: Default::default(),
        },
        reth_primitives::TxType::Eip1559 => TxEnv {
            gas_limit: tx.gas_limit(),
            gas_price: U256::from(tx.max_fee_per_gas()),
            gas_priority_fee: tx.max_priority_fee_per_gas().map(U256::from),
            transact_to: tx.to().into(),
            value: tx.value(),
            data: tx.input().clone(),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            access_list: tx.access_list().unwrap().to_vec(),
            blob_hashes: Default::default(),
            max_fee_per_blob_gas: Default::default(),
            authorization_list: Default::default(),
            caller: sender,
            optimism: Default::default(),
        },
        reth_primitives::TxType::Eip4844 => TxEnv {
            gas_limit: tx.gas_limit(),
            gas_price: U256::from(tx.max_fee_per_gas()),
            gas_priority_fee: tx.max_priority_fee_per_gas().map(U256::from),
            transact_to: tx.to().into(),
            value: tx.value(),
            data: tx.input().clone(),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            access_list: tx.access_list().unwrap().to_vec(),
            blob_hashes: tx.blob_versioned_hashes().unwrap().to_vec(),
            max_fee_per_blob_gas: tx.max_fee_per_blob_gas().map(U256::from),
            authorization_list: Default::default(),
            caller: sender,
            optimism: Default::default(),
        },
        reth_primitives::TxType::Eip7702 => TxEnv {
            gas_limit: tx.gas_limit(),
            gas_price: U256::from(tx.max_fee_per_gas()),
            gas_priority_fee: tx.max_priority_fee_per_gas().map(U256::from),
            transact_to: tx.to().into(),
            value: tx.value(),
            data: tx.input().clone(),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            access_list: tx.access_list().unwrap().to_vec(),
            blob_hashes: Default::default(),
            max_fee_per_blob_gas: Default::default(),
            authorization_list: Some(AuthorizationList::Signed(tx.authorization_list().unwrap().to_vec())),
            caller: sender,
            optimism: Default::default(),
        },
    }
}

pub fn convert_to_alloy_header<N>(header: N::BlockHeader) -> alloy_rpc_types_eth::Header
where
    N: NodePrimitives<BlockHeader = reth_primitives::Header>
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