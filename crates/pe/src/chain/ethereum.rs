use alloy_primitives::ChainId;
use hashbrown::HashMap;
use metis_primitives::{BuildIdentityHasher, hash_deterministic};
use revm::{
    context::{BlockEnv, TxEnv},
    primitives::hardfork::SpecId,
};

use super::{Chain, RewardPolicy};
use crate::{MemoryLocation, TxIdx, mv_memory::MvMemory};

/// Implementation of [`Chain`] for Ethereum
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ethereum {
    id: ChainId,
    spec: SpecId,
}

impl Ethereum {
    /// Ethereum Mainnet
    pub const fn mainnet() -> Self {
        Self {
            id: 1,
            spec: SpecId::PRAGUE,
        }
    }

    /// Custom network
    pub const fn custom(id: ChainId) -> Self {
        Self {
            id,
            spec: SpecId::PRAGUE,
        }
    }
}

impl Chain for Ethereum {
    type Transaction = alloy_rpc_types_eth::Transaction;
    type SpecId = SpecId;

    fn id(&self) -> u64 {
        self.id
    }

    fn spec(&self) -> SpecId {
        self.spec
    }

    fn build_mv_memory(&self, block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
        let block_size = txs.len();
        let beneficiary_location_hash =
            hash_deterministic(MemoryLocation::Basic(block_env.beneficiary));

        // TODO: Estimate more locations based on sender, to and the bytecode static code analysis, etc.
        let mut estimated_locations = HashMap::with_hasher(BuildIdentityHasher::default());
        estimated_locations.insert(
            beneficiary_location_hash,
            (0..block_size).collect::<Vec<TxIdx>>(),
        );

        MvMemory::new(block_size, estimated_locations, [block_env.beneficiary])
    }

    fn get_reward_policy(&self) -> RewardPolicy {
        RewardPolicy::Ethereum
    }

    fn is_eip_1559_enabled(&self, spec_id: SpecId) -> bool {
        spec_id >= SpecId::LONDON
    }

    fn is_eip_161_enabled(&self, spec_id: SpecId) -> bool {
        spec_id >= SpecId::SPURIOUS_DRAGON
    }
}
