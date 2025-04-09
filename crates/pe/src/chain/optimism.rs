//! Optimism

use super::{CalculateReceiptRootError, Chain, RewardPolicy};
use crate::{
    BuildIdentityHasher, MemoryLocation, TxExecutionResult, hash_deterministic, mv_memory::MvMemory,
};
use alloy_consensus::Transaction;
use alloy_primitives::{Address, B256, ChainId, U256};
use alloy_rpc_types_eth::{BlockTransactions, Header};
use hashbrown::HashMap;
use op_alloy_consensus::{OpDepositReceipt, OpReceiptEnvelope, OpTxEnvelope, OpTxType};
use op_alloy_network::eip2718::Encodable2718;
use op_revm::OpSpecId;
use revm::context::{BlockEnv, TxEnv};
use revm::primitives::hardfork::SpecId;

/// Implementation of [`Chain`] for Optimism
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Optimism {
    id: ChainId,
    spec: OpSpecId,
}

impl Optimism {
    /// Optimism Mainnet
    pub const fn mainnet() -> Self {
        Self {
            id: 10,
            spec: OpSpecId::ISTHMUS,
        }
    }

    /// Custom network
    pub const fn custom(id: ChainId) -> Self {
        Self {
            id,
            spec: OpSpecId::ISTHMUS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptimismBlockSpecError {
    #[error("Spec is not supported")]
    UnsupportedSpec,
}

/// Represents errors that can occur when parsing transactions
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptimismTransactionParsingError {
    #[error("Transaction must set gas price")]
    MissingGasPrice,
}

fn get_optimism_gas_price(tx: &OpTxEnvelope) -> Result<U256, OptimismTransactionParsingError> {
    match tx.tx_type() {
        OpTxType::Legacy | OpTxType::Eip2930 => tx
            .gas_price()
            .map(U256::from)
            .ok_or(OptimismTransactionParsingError::MissingGasPrice),
        OpTxType::Eip1559 | OpTxType::Eip7702 => Ok(U256::from(tx.max_fee_per_gas())),
        OpTxType::Deposit => Ok(U256::ZERO),
    }
}

impl Chain for Optimism {
    type Transaction = op_alloy_rpc_types::Transaction;
    type Envelope = OpTxEnvelope;
    type BlockSpecError = OptimismBlockSpecError;
    type TransactionParsingError = OptimismTransactionParsingError;
    type SpecId = OpSpecId;

    fn id(&self) -> u64 {
        self.id
    }

    fn spec(&self) -> SpecId {
        self.spec.into_eth_spec()
    }

    fn get_block_spec(&self, header: &Header) -> Result<OpSpecId, Self::BlockSpecError> {
        // TODO: The implementation below is only true for Optimism Mainnet.
        // When supporting other networks (e.g. Optimism Sepolia), remember to adjust the code here.
        if header.timestamp >= 1720627201 {
            Ok(OpSpecId::FJORD)
        } else if header.timestamp >= 1710374401 {
            Ok(OpSpecId::ECOTONE)
        } else if header.timestamp >= 1704992401 {
            Ok(OpSpecId::CANYON)
        } else if header.number >= 105235063 {
            // On Optimism Mainnet, Bedrock and Regolith are activated at the same time.
            // Therefore, this function never returns SpecId::BEDROCK.
            // The statement above might not be true for other networks, e.g. Optimism Goerli.
            Ok(OpSpecId::REGOLITH)
        } else {
            // TODO: revm does not support pre-Bedrock blocks.
            // https://docs.optimism.io/builders/node-operators/architecture#legacy-geth
            Err(OptimismBlockSpecError::UnsupportedSpec)
        }
    }

    fn build_mv_memory(&self, block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
        let beneficiary_location_hash =
            hash_deterministic(MemoryLocation::Basic(block_env.beneficiary));
        let l1_fee_recipient_location_hash =
            hash_deterministic(op_revm::constants::L1_FEE_RECIPIENT);
        let base_fee_recipient_location_hash =
            hash_deterministic(op_revm::constants::BASE_FEE_RECIPIENT);

        // TODO: Estimate more locations based on sender, to, etc.
        let mut estimated_locations = HashMap::with_hasher(BuildIdentityHasher::default());
        for (index, tx) in txs.iter().enumerate() {
            // TODO: Benchmark to check whether adding these estimated
            // locations helps or harms the performance.
            estimated_locations
                .entry(l1_fee_recipient_location_hash)
                .or_insert_with(|| Vec::with_capacity(1))
                .push(index);
            estimated_locations
                .entry(base_fee_recipient_location_hash)
                .or_insert_with(|| Vec::with_capacity(1))
                .push(index);
        }

        MvMemory::new(
            txs.len(),
            estimated_locations,
            [
                block_env.beneficiary,
                op_revm::constants::L1_FEE_RECIPIENT,
                op_revm::constants::BASE_FEE_RECIPIENT,
            ],
        )
    }

    fn get_reward_policy(&self) -> RewardPolicy {
        RewardPolicy::Optimism {
            l1_fee_recipient_location_hash: hash_deterministic(MemoryLocation::Basic(
                op_revm::constants::L1_FEE_RECIPIENT,
            )),
            base_fee_vault_location_hash: hash_deterministic(MemoryLocation::Basic(
                op_revm::constants::BASE_FEE_RECIPIENT,
            )),
        }
    }

    fn is_eip_1559_enabled(&self, _spec_id: SpecId) -> bool {
        true
    }

    fn is_eip_161_enabled(&self, _spec_id: SpecId) -> bool {
        true
    }
}
