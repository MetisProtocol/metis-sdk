//! Chain specific utils

use revm::{
    context::{BlockEnv, TxEnv},
    primitives::hardfork::SpecId,
};
use std::fmt::Debug;

use crate::mv_memory::MvMemory;

/// Different chains may have varying reward policies.
/// This enum specifies which policy to follow, with optional
/// pre-calculated data to assist in reward calculations.
#[derive(Debug, Clone)]
pub enum RewardPolicy {
    /// Ethereum
    Ethereum,
    /// Optimism
    #[cfg(feature = "optimism")]
    Optimism {
        /// L1 Fee Recipient
        l1_fee_recipient_location_hash: crate::MemoryLocationHash,
        /// Base Fee Vault
        base_fee_vault_location_hash: crate::MemoryLocationHash,
    },
}

/// Custom behaviours for different chains & networks
pub trait Chain: Debug {
    /// The transaction type
    type Transaction: Debug + Clone + PartialEq;

    /// The chain spec id
    type SpecId;

    /// Get chain id.
    fn id(&self) -> u64;

    fn spec(&self) -> SpecId;

    /// Build [`MvMemory`]
    fn build_mv_memory(&self, _block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
        MvMemory::new(txs.len(), [], [])
    }

    /// Get [`RewardPolicy`]
    fn get_reward_policy(&self) -> RewardPolicy;

    /// Check whether EIP-1559 is enabled
    /// <https://github.com/ethereum/EIPs/blob/96523ef4d76ca440f73f0403ddb5c9cb3b24dcae/EIPS/eip-1559.md>
    fn is_eip_1559_enabled(&self, spec_id: SpecId) -> bool;

    /// Check whether EIP-161 is enabled
    /// <https://github.com/ethereum/EIPs/blob/96523ef4d76ca440f73f0403ddb5c9cb3b24dcae/EIPS/eip-161.md>
    fn is_eip_161_enabled(&self, spec_id: SpecId) -> bool;
}

mod ethereum;
pub use ethereum::Ethereum;

#[cfg(feature = "optimism")]
mod optimism;
#[cfg(feature = "optimism")]
pub use optimism::Optimism;
