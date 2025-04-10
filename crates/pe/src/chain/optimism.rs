use super::{Chain, RewardPolicy};
use crate::{MemoryLocation, mv_memory::MvMemory};
use alloy_primitives::ChainId;
use hashbrown::HashMap;
use metis_primitives::{BuildIdentityHasher, hash_deterministic};
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

impl Chain for Optimism {
    type Transaction = op_alloy_rpc_types::Transaction;
    type SpecId = OpSpecId;

    fn id(&self) -> u64 {
        self.id
    }

    fn spec(&self) -> SpecId {
        self.spec.into_eth_spec()
    }

    fn build_mv_memory(&self, block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
        let _beneficiary_location_hash =
            hash_deterministic(MemoryLocation::Basic(block_env.beneficiary));
        let l1_fee_recipient_location_hash =
            hash_deterministic(op_revm::constants::L1_FEE_RECIPIENT);
        let base_fee_recipient_location_hash =
            hash_deterministic(op_revm::constants::BASE_FEE_RECIPIENT);

        // TODO: Estimate more locations based on sender, to, etc.
        let mut estimated_locations = HashMap::with_hasher(BuildIdentityHasher::default());
        for (index, _tx) in txs.iter().enumerate() {
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
