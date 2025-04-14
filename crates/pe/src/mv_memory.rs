use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Mutex,
};

use alloy_primitives::{Address, B256};
use dashmap::DashMap;
use hashbrown::HashMap;
use metis_primitives::{BuildIdentityHasher, BuildSuffixHasher, hash_deterministic};
use revm::bytecode::Bytecode;
use revm::context::{BlockEnv, TxEnv};
use std::fmt::Debug;

use crate::{
    MemoryEntry, MemoryLocation, MemoryLocationHash, ReadOrigin, ReadSet, TxIdx, TxVersion,
    WriteSet,
};

#[derive(Default, Debug)]
struct LastLocations {
    read: ReadSet,
    // Consider [SmallVec] since most transactions explicitly write to 2 locations!
    write: Vec<MemoryLocationHash>,
}

type LazyAddresses = HashSet<Address, BuildSuffixHasher>;

/// The `MvMemory` contains shared memory in a form of a multi-version data
/// structure for values written and read by different transactions. It stores
/// multiple writes for each memory location, along with a value and an associated
/// version of a corresponding transaction.
#[derive(Debug)]
pub struct MvMemory {
    /// The list of transaction incarnations and written values for each memory location
    // No more hashing is required as we already identify memory locations by their hash
    // in the read & write sets. [dashmap] having a dedicated interface for this use case
    // (that skips hashing for [u64] keys) would make our code cleaner and "faster".
    // Nevertheless, the compiler should be good enough to optimize these cases anyway.
    pub(crate) data: DashMap<MemoryLocationHash, BTreeMap<TxIdx, MemoryEntry>, BuildIdentityHasher>,
    // list of transactions that reaad the memory location
    pub(crate) location_reads: DashMap<MemoryLocationHash, BTreeSet<TxIdx>, BuildIdentityHasher>,
    /// Last read & written locations of each transaction
    last_locations: Vec<Mutex<LastLocations>>,
    /// Lazy addresses that need full evaluation at the end of the block
    lazy_addresses: Mutex<LazyAddresses>,
    /// New bytecodes deployed in this block
    pub(crate) new_bytecodes: DashMap<B256, Bytecode, BuildSuffixHasher>,
}

impl MvMemory {
    pub(crate) fn new(
        block_size: usize,
        estimated_locations: impl IntoIterator<Item = (MemoryLocationHash, Vec<TxIdx>)>,
        lazy_addresses: impl IntoIterator<Item = Address>,
    ) -> Self {
        // TODO: Fine-tune the number of shards, like to the next number of two from the
        // number of worker threads.
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
                    .map(|tx_idx| (tx_idx, MemoryEntry::Estimate))
                    .collect(),
            );
        }
        Self {
            data,
            location_reads: DashMap::default(),
            last_locations: (0..block_size).map(|_| Mutex::default()).collect(),
            lazy_addresses: Mutex::new(LazyAddresses::from_iter(lazy_addresses)),
            // TODO: Fine-tune the number of shards, like to the next number of two from the
            // number of worker threads.
            new_bytecodes: DashMap::default(),
        }
    }

    pub(crate) fn add_lazy_addresses(&self, new_lazy_addresses: impl IntoIterator<Item = Address>) {
        let mut lazy_addresses = self.lazy_addresses.lock().unwrap();
        for address in new_lazy_addresses {
            lazy_addresses.insert(address);
        }
    }

    // Apply a new pair of read & write sets to the multi-version data structure.
    // Return whether a write occurred to a memory location not written to by
    // the previous incarnation of the same transaction. This determines whether
    // the executed higher transactions need re-validation.
    pub(crate) fn record(
        &self,
        tx_version: &TxVersion,
        read_set: ReadSet,
        write_set: WriteSet,
    ) -> Vec<TxIdx> {
        // update location_reads
        for location in read_set.keys() {
            self.location_reads
                .entry(*location)
                .or_default()
                .insert(tx_version.tx_idx);
        }

        let mut last_locations = index_mutex!(self.last_locations, tx_version.tx_idx);
        for location in last_locations
            .read
            .keys()
            .filter(|k| !read_set.contains_key(*k))
        {
            self.location_reads
                .get_mut(location)
                .unwrap()
                .remove(&tx_version.tx_idx);
        }
        last_locations.read = read_set;

        // TODO2: DOCs
        let mut changed_locations = Vec::new();

        // TODO: Group updates by shard to avoid locking operations.
        // Remove old locations that aren't written to anymore.
        // TODO2: USE hashset for write_set
        let mut last_location_idx = 0;
        while last_location_idx < last_locations.write.len() {
            let prev_location = unsafe { last_locations.write.get_unchecked(last_location_idx) };
            if !write_set.contains_key(prev_location) {
                if let Some(mut written_transactions) = self.data.get_mut(prev_location) {
                    written_transactions.remove(&tx_version.tx_idx);
                }
                // add removed locations
                changed_locations.push(*prev_location);
                // remove location
                last_locations.write.swap_remove(last_location_idx);
            } else {
                last_location_idx += 1;
            }
        }

        for (location, value) in write_set {
            if let Some(MemoryEntry::Data(_, old_value)) =
                self.data.entry(location).or_default().insert(
                    tx_version.tx_idx,
                    MemoryEntry::Data(tx_version.tx_incarnation, value.clone()),
                )
            {
                if old_value == value {
                    continue;
                }
            }
            changed_locations.push(location);
        }

        let affected_txs: HashSet<TxIdx> = changed_locations
            .iter()
            .flat_map(|l| {
                if let Some(txid_set) = self.location_reads.get(l) {
                    if let Some(written_transactions) = self.data.get(l) {
                        if let Some((txid, _)) =
                            written_transactions.range(tx_version.tx_idx + 1..).next()
                        {
                            return txid_set
                                .range(tx_version.tx_idx + 1..txid + 1)
                                .cloned()
                                .collect();
                        }
                    }
                    txid_set.range(tx_version.tx_idx + 1..).cloned().collect()
                } else {
                    Vec::new()
                }
            })
            .collect();

        affected_txs.into_iter().collect()
    }

    // Obtain the last read set recorded by an execution of [tx_idx] and check
    // that re-reading each memory location in the read set still yields the
    // same read origins.
    // This is invoked during validation, when the incarnation being validated is
    // already executed and has recorded the read set. However, if the thread
    // performing a validation for incarnation i of a transaction is slow, it is
    // possible that this function invocation observes a read set recorded by a
    // latter (> i) incarnation. In this case, incarnation i is guaranteed to be
    // already aborted (else higher incarnations would never start), and the
    // validation task will have no effect regardless of the outcome (only
    // validations that successfully abort affect the state and each incarnation
    // can be aborted at most once).
    pub(crate) fn validate_read_locations(&self, tx_idx: TxIdx) -> bool {
        for (location, prior_origins) in &index_mutex!(self.last_locations, tx_idx).read {
            if let Some(written_transactions) = self.data.get(location) {
                let mut iter = written_transactions.range(..tx_idx);
                for prior_origin in prior_origins {
                    if let ReadOrigin::MvMemory(prior_version) = prior_origin {
                        // Found something: Must match version.
                        if let Some((closest_idx, MemoryEntry::Data(tx_incarnation, ..))) =
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

    pub(crate) fn consume_lazy_addresses(&self) -> impl IntoIterator<Item = Address> {
        std::mem::take(&mut *self.lazy_addresses.lock().unwrap()).into_iter()
    }
}

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

#[cfg(feature = "optimism")]
#[inline]
pub fn reward_policy() -> RewardPolicy {
    RewardPolicy::Optimism {
        l1_fee_recipient_location_hash: hash_deterministic(MemoryLocation::Basic(
            op_revm::constants::L1_FEE_RECIPIENT,
        )),
        base_fee_vault_location_hash: hash_deterministic(MemoryLocation::Basic(
            op_revm::constants::BASE_FEE_RECIPIENT,
        )),
    }
}

#[cfg(not(feature = "optimism"))]
#[inline]
pub fn reward_policy() -> RewardPolicy {
    RewardPolicy::Ethereum
}

#[cfg(feature = "optimism")]
pub fn build_mv_memory(block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
    let _beneficiary_location_hash =
        hash_deterministic(MemoryLocation::Basic(block_env.beneficiary));
    let l1_fee_recipient_location_hash = hash_deterministic(op_revm::constants::L1_FEE_RECIPIENT);
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

#[cfg(not(feature = "optimism"))]
pub fn build_mv_memory(block_env: &BlockEnv, txs: &[TxEnv]) -> MvMemory {
    use crate::TxIdx;

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
