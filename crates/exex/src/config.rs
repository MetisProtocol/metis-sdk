use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_ethereum_forks::{EthereumHardfork, Head};
use revm::primitives::SpecId;

/// Returns the revm [`SpecId`](revm_primitives::SpecId) at the given timestamp.
///
/// # Note
///
/// This is only intended to be used after the merge, when hardforks are activated by
/// timestamp.
pub fn revm_spec_by_timestamp_after_merge(
    chain_spec: &ChainSpec,
    timestamp: u64,
) -> SpecId {
    if chain_spec.is_prague_active_at_timestamp(timestamp) {
        revm::primitives::PRAGUE
    } else if chain_spec.is_cancun_active_at_timestamp(timestamp) {
        revm::primitives::CANCUN
    } else if chain_spec.is_shanghai_active_at_timestamp(timestamp) {
        revm::primitives::SHANGHAI
    } else {
        revm::primitives::MERGE
    }
}

/// Map the latest active hardfork at the given block to a revm [`SpecId`](revm_primitives::SpecId).
pub fn revm_spec(chain_spec: &ChainSpec, block: &Head) -> SpecId {
    if chain_spec.fork(EthereumHardfork::Prague).active_at_head(block) {
        revm::primitives::PRAGUE
    } else if chain_spec.fork(EthereumHardfork::Cancun).active_at_head(block) {
        revm::primitives::CANCUN
    } else if chain_spec.fork(EthereumHardfork::Shanghai).active_at_head(block) {
        revm::primitives::SHANGHAI
    } else if chain_spec.fork(EthereumHardfork::Paris).active_at_head(block) {
        revm::primitives::MERGE
    } else if chain_spec.fork(EthereumHardfork::London).active_at_head(block) {
        revm::primitives::LONDON
    } else if chain_spec.fork(EthereumHardfork::Berlin).active_at_head(block) {
        revm::primitives::BERLIN
    } else if chain_spec.fork(EthereumHardfork::Istanbul).active_at_head(block) {
        revm::primitives::ISTANBUL
    } else if chain_spec.fork(EthereumHardfork::Petersburg).active_at_head(block) {
        revm::primitives::PETERSBURG
    } else if chain_spec.fork(EthereumHardfork::Byzantium).active_at_head(block) {
        revm::primitives::BYZANTIUM
    } else if chain_spec.fork(EthereumHardfork::SpuriousDragon).active_at_head(block) {
        revm::primitives::SPURIOUS_DRAGON
    } else if chain_spec.fork(EthereumHardfork::Tangerine).active_at_head(block) {
        revm::primitives::TANGERINE
    } else if chain_spec.fork(EthereumHardfork::Homestead).active_at_head(block) {
        revm::primitives::HOMESTEAD
    } else if chain_spec.fork(EthereumHardfork::Frontier).active_at_head(block) {
        revm::primitives::FRONTIER
    } else {
        panic!(
            "invalid hardfork chainspec: expected at least one hardfork, got {:?}",
            chain_spec.hardforks
        )
    }
}
