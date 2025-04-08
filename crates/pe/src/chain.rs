//! Chain specific utils

use std::error::Error as StdError;
use std::fmt::Debug;

use alloy_consensus::{Signed, TxLegacy};
use alloy_primitives::B256;
use alloy_rpc_types_eth::{BlockTransactions, Header};
use revm::{
    context::{BlockEnv, TxEnv},
    primitives::hardfork::SpecId,
};

use crate::{TxExecutionResult, mv_memory::MvMemory};

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

/// The error type of [`Chain::calculate_receipt_root`]
#[derive(Debug, Clone)]
pub enum CalculateReceiptRootError {
    /// Unsupported
    Unsupported,
    /// Invalid transaction type
    InvalidTxType(u8),
    /// Arbitrary error message
    Custom(String),
    /// Optimism deposit is missing sender
    #[cfg(feature = "optimism")]
    OpDepositMissingSender,
}

/// Custom behaviours for different chains & networks
pub trait Chain: Debug {
    /// The transaction type
    type Transaction: Debug + Clone + PartialEq;

    /// The envelope type
    // TODO: Support more tx conversions
    type Envelope: Debug + From<Signed<TxLegacy>>;

    /// The error type for [`Self::get_block_spec`].
    type BlockSpecError: StdError + Debug + Clone + PartialEq + 'static;

    /// The error type for [`Self::get_tx_env`].
    type TransactionParsingError: StdError + Debug + Clone + PartialEq + 'static;

    /// The chain spec id
    type SpecId;

    /// Get chain id.
    fn id(&self) -> u64;

    fn spec(&self) -> SpecId;

    /// Get block's [`SpecId`]
    fn get_block_spec(&self, header: &Header) -> Result<Self::SpecId, Self::BlockSpecError>;

    /// Build [`MvMemory`]
    fn build_mv_memory(&self, _block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
        MvMemory::new(txs.len(), [], [])
    }

    /// Get [`RewardPolicy`]
    fn get_reward_policy(&self) -> RewardPolicy;

    /// Calculate receipt root
    fn calculate_receipt_root(
        &self,
        spec_id: Self::SpecId,
        txs: &BlockTransactions<Self::Transaction>,
        tx_results: &[TxExecutionResult],
    ) -> Result<B256, CalculateReceiptRootError>;

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
