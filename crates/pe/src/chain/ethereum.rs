//! Ethereum

use alloy_consensus::TxEnvelope;
use alloy_primitives::ChainId;
use alloy_rpc_types_eth::Header;
use hashbrown::HashMap;
use revm::{
    context::{BlockEnv, TxEnv},
    primitives::hardfork::SpecId,
};

use super::{Chain, RewardPolicy};
use crate::{BuildIdentityHasher, MemoryLocation, TxIdx, hash_deterministic, mv_memory::MvMemory};

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

/// Represents errors that can occur when parsing transactions
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EthereumTransactionParsingError {
    /// [`tx.gas_price`] is none.
    #[error("Missing gas price")]
    MissingGasPrice,
}

impl Chain for Ethereum {
    type Transaction = alloy_rpc_types_eth::Transaction;
    type Envelope = TxEnvelope;
    type BlockSpecError = std::convert::Infallible;
    type TransactionParsingError = EthereumTransactionParsingError;
    type SpecId = SpecId;

    fn id(&self) -> u64 {
        self.id
    }

    fn spec(&self) -> SpecId {
        self.spec
    }

    /// Get the REVM spec id of an Alloy block.
    // Currently hardcoding Ethereum hardforks from these references:
    // https://github.com/paradigmxyz/reth/blob/4fa627736681289ba899b38f1c7a97d9fcf33dc6/crates/primitives/src/revm/config.rs#L33-L78
    // https://github.com/paradigmxyz/reth/blob/4fa627736681289ba899b38f1c7a97d9fcf33dc6/crates/primitives/src/chain/spec.rs#L44-L68
    // TODO: Better error handling & properly test this.
    // TODO: Only Ethereum Mainnet is supported at the moment.
    fn get_block_spec(&self, header: &Header) -> Result<SpecId, Self::BlockSpecError> {
        Ok(if header.timestamp >= 1710338135 {
            SpecId::CANCUN
        } else if header.timestamp >= 1681338455 {
            SpecId::SHANGHAI
        }
        // Checking for total difficulty is more precise but many RPC providers stopped returning it...
        else if header.number >= 15537394 {
            SpecId::MERGE
        } else if header.number >= 12965000 {
            SpecId::LONDON
        } else if header.number >= 12244000 {
            SpecId::BERLIN
        } else if header.number >= 9069000 {
            SpecId::ISTANBUL
        } else if header.number >= 7280000 {
            SpecId::PETERSBURG
        } else if header.number >= 4370000 {
            SpecId::BYZANTIUM
        } else if header.number >= 2675000 {
            SpecId::SPURIOUS_DRAGON
        } else if header.number >= 2463000 {
            SpecId::TANGERINE
        } else if header.number >= 1150000 {
            SpecId::HOMESTEAD
        } else {
            SpecId::FRONTIER
        })
    }

    fn build_mv_memory(&self, block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
        let block_size = txs.len();
        let beneficiary_location_hash =
            hash_deterministic(MemoryLocation::Basic(block_env.beneficiary));

        // TODO: Estimate more locations based on sender, to, etc.
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
