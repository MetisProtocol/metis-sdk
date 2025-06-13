use std::{collections::BTreeMap, fmt::Debug, sync::Mutex};

use alloy_primitives::{Address, B256};
use dashmap::{DashMap, DashSet};
use metis_primitives::{BuildIdentityHasher, BuildSuffixHasher, HashMap, hash_deterministic};
use revm::{
    bytecode::Bytecode,
    context::{BlockEnv, TxEnv},
};

use crate::{
    Entry, Location, LocationHash, ReadOrigin, ReadSet, ReadWriteSet, TxIdx, TxVersion, WriteSet,
};

/// The [`MvMemory`] contains shared memory in a form of a multi-version data
/// structure for values written and read by different transactions. It stores
/// multiple writes for each location, along with a value and an associated
/// version of a corresponding transaction.
#[derive(Debug)]
pub struct MvMemory {
    /// The list of transaction incarnations and written values for each location
    /// No more hashing is required as we already identify locations by their hash
    /// in the read & write sets. [dashmap] having a dedicated interface for this use case
    /// (that skips hashing for [u64] keys) would make our code cleaner and "faster".
    pub(crate) data: DashMap<LocationHash, BTreeMap<TxIdx, Entry>, BuildIdentityHasher>,
    /// List of read & written locations of each transaction
    locations: Vec<Mutex<ReadWriteSet>>,
    /// Lazy addresses that need full evaluation at the end of the block
    lazy_addresses: DashSet<Address, BuildSuffixHasher>,
    /// New bytecodes deployed in this block
    pub(crate) new_bytecodes: DashMap<B256, Bytecode, BuildSuffixHasher>,
}

impl MvMemory {
    pub(crate) fn new(
        block_size: usize,
        estimated_locations: impl IntoIterator<Item = (LocationHash, Vec<TxIdx>)>,
        lazy_addresses: impl IntoIterator<Item = Address>,
    ) -> Self {
        let data = DashMap::default();
        // We preallocate estimated locations to avoid restructuring trees at runtime
        // while holding a write lock. Ideally [dashmap] would have a lock-free
        // construction API. This is acceptable for now as it's a non-congested one-time
        // cost.
        for (location_hash, estimated_tx_idxs) in estimated_locations {
            data.insert(
                location_hash,
                estimated_tx_idxs
                    .into_iter()
                    .map(|tx_idx| (tx_idx, Entry::Estimate))
                    .collect(),
            );
        }
        Self {
            data,
            locations: (0..block_size).map(|_| Mutex::default()).collect(),
            lazy_addresses: DashSet::from_iter(lazy_addresses),
            new_bytecodes: DashMap::default(),
        }
    }

    #[inline]
    pub(crate) fn add_lazy_addresses(&self, new_lazy_addresses: impl IntoIterator<Item = Address>) {
        for address in new_lazy_addresses {
            self.lazy_addresses.insert(address);
        }
    }

    /// Record the pair of read & write sets to the multi-version data structure.
    /// Return whether a write occurred to a location not written to by
    /// the previous incarnation of the same transaction. This determines whether
    /// the executed higher transactions need re-validation.
    pub(crate) fn record(
        &self,
        tx_version: &TxVersion,
        read_set: ReadSet,
        write_set: WriteSet,
    ) -> bool {
        // Update read set
        let mut locations = index_mutex!(self.locations, tx_version.tx_idx);
        locations.read = read_set;

        let mut changed_locations = Vec::new();

        // Update write set and remove old locations that aren't written to anymore.
        let mut location_idx = 0;
        while location_idx < locations.write.len() {
            let prev_location = unsafe { locations.write.get_unchecked(location_idx) };
            if !write_set.contains_key(prev_location) {
                if let Some(mut written_txs) = self.data.get_mut(prev_location) {
                    written_txs.remove(&tx_version.tx_idx);
                }
                // add removed locations
                changed_locations.push(*prev_location);
                // remove location
                locations.write.swap_remove(location_idx);
            } else {
                location_idx += 1;
            }
        }

        // Register new writes.
        let mut wrote_new_location = false;

        for (location, value) in write_set {
            self.data.entry(location).or_default().insert(
                tx_version.tx_idx,
                Entry::Data(tx_version.tx_incarnation, value),
            );
            if !locations.write.contains(&location) {
                locations.write.push(location);
                wrote_new_location = true;
            }
        }

        wrote_new_location
    }

    /// Obtain the read set recorded by an execution of [tx_idx] and check
    /// that re-reading each location in the read set still yields the
    /// same read origins.
    /// This is invoked during validation, when the incarnation being validated is
    /// already executed and has recorded the read set. However, if the thread
    /// performing a validation for incarnation i of a transaction is slow, it is
    /// possible that this function invocation observes a read set recorded by a
    /// latter (> i) incarnation. In this case, incarnation i is guaranteed to be
    /// already aborted (else higher incarnations would never start), and the
    /// validation task will have no effect regardless of the outcome (only
    /// validations that successfully abort affect the state and each incarnation
    /// can be aborted at most once).
    pub(crate) fn validate_read_locations(&self, tx_idx: TxIdx) -> bool {
        for (location, prior_origins) in &index_mutex!(self.locations, tx_idx).read {
            if let Some(written_txs) = self.data.get(location) {
                let mut iter = written_txs.range(..tx_idx);
                for prior_origin in prior_origins {
                    if let ReadOrigin::MvMemory(prior_version) = prior_origin {
                        // Found something: Must match version.
                        if let Some((closest_idx, Entry::Data(tx_incarnation, ..))) =
                            iter.next_back()
                        {
                            if closest_idx != &prior_version.tx_idx
                                || &prior_version.tx_incarnation != tx_incarnation
                            {
                                return false;
                            }
                        }
                        // The previously read value is now cleared
                        // or marked with ESTIMATE.
                        else {
                            return false;
                        }
                    } else if iter.next_back().is_some() {
                        return false;
                    }
                }
            }
            // Read from multi-version data but now it's cleared.
            else if prior_origins.len() != 1 || prior_origins.last() != Some(&ReadOrigin::Storage)
            {
                return false;
            }
        }
        true
    }

    #[inline]
    pub(crate) fn convert_writes_to_estimates(&self, tx_idx: TxIdx) {
        for location in &index_mutex!(self.locations, tx_idx).write {
            if let Some(mut written_txs) = self.data.get_mut(location) {
                written_txs.insert(tx_idx, Entry::Estimate);
            }
        }
    }

    #[inline]
    pub(crate) fn consume_lazy_addresses(&self) -> impl IntoIterator<Item = Address> {
        self.lazy_addresses.clone().into_iter()
    }
}

/// Different chains may have varying reward policies.
#[derive(Debug, Clone)]
pub enum RewardPolicy {
    /// Ethereum
    Ethereum,
}

/// Optimism chain reward policies.
#[derive(Debug, Clone)]
pub enum OpRewardPolicy {
    /// Optimism
    Optimism {
        /// L1 Fee Recipient
        l1_fee_recipient_location_hash: crate::LocationHash,
        /// Base Fee Vault
        base_fee_vault_location_hash: crate::LocationHash,
    },
}

#[inline]
pub fn op_reward_policy() -> OpRewardPolicy {
    OpRewardPolicy::Optimism {
        l1_fee_recipient_location_hash: hash_deterministic(Location::Basic(
            op_revm::constants::L1_FEE_RECIPIENT,
        )),
        base_fee_vault_location_hash: hash_deterministic(Location::Basic(
            op_revm::constants::BASE_FEE_RECIPIENT,
        )),
    }
}

#[inline]
pub fn reward_policy() -> RewardPolicy {
    RewardPolicy::Ethereum
}

pub fn build_op_mv_memory(block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
    let _beneficiary_location_hash = hash_deterministic(Location::Basic(block_env.beneficiary));
    let l1_fee_recipient_location_hash = hash_deterministic(op_revm::constants::L1_FEE_RECIPIENT);
    let base_fee_recipient_location_hash =
        hash_deterministic(op_revm::constants::BASE_FEE_RECIPIENT);

    // TODO: Estimate more locations based on sender, to, etc.
    let mut estimated_locations = HashMap::with_hasher(BuildIdentityHasher::default());
    for (index, _tx) in txs.iter().enumerate() {
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

pub fn build_mv_memory(block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
    use crate::TxIdx;

    let block_size = txs.len();
    let beneficiary_location_hash = hash_deterministic(Location::Basic(block_env.beneficiary));

    // TODO: Estimate more locations based on sender, to and the bytecode static code analysis, etc.
    let mut estimated_locations = HashMap::with_hasher(BuildIdentityHasher::default());
    estimated_locations.insert(
        beneficiary_location_hash,
        (0..block_size).collect::<Vec<TxIdx>>(),
    );

    MvMemory::new(block_size, estimated_locations, [block_env.beneficiary])
}
