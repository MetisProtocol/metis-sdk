//! Reference: https://github.com/risechain/pevm

use alloy_primitives::{Address, B256, U256};
use hashbrown::HashMap;
use metis_primitives::BuildIdentityHasher;
use revm::state::AccountInfo;
use smallvec::SmallVec;
use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct AtomicWrapper<T: Into<usize> + From<usize>> {
    inner: AtomicUsize,
    _marker: PhantomData<T>,
}

impl<T: Into<usize> + From<usize>> AtomicWrapper<T> {
    #[inline]
    pub fn new(value: T) -> Self {
        Self {
            inner: AtomicUsize::new(value.into()),
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn load(&self, ordering: Ordering) -> T {
        T::from(self.inner.load(ordering))
    }

    #[inline]
    pub fn store(&self, value: T, ordering: Ordering) {
        self.inner.store(value.into(), ordering);
    }

    #[inline]
    pub fn swap(&self, value: T, ordering: Ordering) -> T {
        T::from(self.inner.swap(value.into(), ordering))
    }

    #[inline]
    pub fn compare_exchange(
        &self,
        current: T,
        new: T,
        success: Ordering,
        failure: Ordering,
    ) -> Result<T, T> {
        self.inner
            .compare_exchange(current.into(), new.into(), success, failure)
            .map(T::from)
            .map_err(T::from)
    }
}

impl<T: Into<usize> + From<usize> + fmt::Debug> fmt::Debug for AtomicWrapper<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.load(Ordering::SeqCst).fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MemoryLocation {
    // TODO: Separate an account's balance and nonce?
    Basic(Address),
    CodeHash(Address),
    Storage(Address, U256),
}

/// We only need the full memory location to read from storage.
/// We then identify the locations with its hash in the multi-version
/// data, write and read sets, which is much faster than rehashing
/// on every single lookup & validation.
pub type MemoryLocationHash = u64;

// TODO: It would be nice if we could tie the different cases of
// memory locations & values at the type level, to prevent lots of
// matches & potentially dangerous mismatch mistakes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryValue {
    Basic(AccountInfo),
    CodeHash(B256),
    Storage(U256),
    // We lazily update the beneficiary balance to avoid continuous
    // dependencies as all transactions read and write to it. We also
    // lazy update the senders & recipients of raw transfers, which are
    // also common (popular CEX addresses, airdrops, etc).
    // We fully evaluate these account states at the end of the block or
    // when there is an explicit read.
    // Explicit balance addition.
    LazyRecipient(U256),
    // Explicit balance subtraction & implicit nonce increment.
    LazySender(U256),
    // The account was self-destructed.
    SelfDestructed,
}

#[derive(Debug)]
pub enum MemoryEntry {
    Data(TxIncarnation, MemoryValue),
    // When an incarnation is aborted due to a validation failure, the
    // entries in the multi-version data structure corresponding to its
    // write set are replaced with this special ESTIMATE marker.
    // This signifies that the next incarnation is estimated to write to
    // the same memory locations. An incarnation stops and is immediately
    // aborted whenever it reads a value marked as an ESTIMATE written by
    // a lower transaction, instead of potentially wasting a full execution
    // and aborting during validation.
    // The ESTIMATE markers that are not overwritten are removed by the next
    // incarnation.
    Estimate,
}

/// The index of the transaction in the block.
pub type TxIdx = usize;

/// The i-th time a transaction is re-executed, counting from 0.
pub type TxIncarnation = usize;

/// Execution:
/// - ReadyToExecute(i) --try_incarnate--> Executing(i)
///
/// Non-blocked execution:
/// - Executing(i) --finish_execution--> Executed(i)
/// - Executed(i) --finish_validation--> Validated(i)
/// - Executed/Validated(i) --try_validation_abort--> Aborting(i)
/// - Aborted(i) --finish_validation(w.aborted=true)--> ReadyToExecute(i+1)
///
/// Blocked execution:
/// - Executing(i) --add_dependency--> Aborting(i)
/// - Blocking(i) --resume--> ReadyToExecute(i+1)
#[derive(PartialEq, Clone, Debug)]
pub enum IncarnationStatus {
    ReadyToExecute = 0,
    Executing,
    Executed,
    Validated,
    Blocking,
}

#[derive(PartialEq, Clone, Debug)]
pub struct TxStatus {
    pub incarnation: TxIncarnation,
    pub status: IncarnationStatus,
}

impl From<usize> for TxStatus {
    fn from(value: usize) -> Self {
        let incarnation = value >> 8;
        let status = match value & 0xFF {
            0 => IncarnationStatus::ReadyToExecute,
            1 => IncarnationStatus::Executing,
            2 => IncarnationStatus::Executed,
            3 => IncarnationStatus::Validated,
            _ => IncarnationStatus::Blocking,
        };
        Self {
            incarnation,
            status,
        }
    }
}

impl From<TxStatus> for usize {
    fn from(val: TxStatus) -> Self {
        (val.incarnation << 8) | val.status as usize
    }
}

/// We maintain an in-memory multi-version data structure that stores for
/// each memory location the latest value written per transaction, along
/// with the associated transaction incarnation. When a transaction reads
/// a memory location, it obtains from the multi-version data structure the
/// value written to this location by the highest transaction that appears
/// before it in the block, along with the associated version. If no previous
/// transactions have written to a location, the value would be read from the
/// storage state before block execution.
#[derive(Clone, Debug, PartialEq)]
pub struct TxVersion {
    pub tx_idx: TxIdx,
    pub tx_incarnation: TxIncarnation,
}

/// The origin of a memory read. It could be from the live multi-version
/// data structure or from storage (chain state before block execution).
#[derive(Debug, PartialEq)]
pub enum ReadOrigin {
    MvMemory(TxVersion),
    Storage,
}

/// A scheduled worker task
/// TODO: Add more useful work when there are idle workers like near
/// the end of block execution, while waiting for a huge blocking
/// transaction to resolve, etc.
#[derive(Debug)]
pub enum Task {
    Execution(TxVersion),
    Validation(TxIdx),
}

/// Most memory locations only have one read origin. Lazy updated ones like
/// the beneficiary balance, raw transfer senders & recipients, etc. have a
/// list of lazy updates all the way to the first strict/absolute value.
pub type ReadOrigins = SmallVec<[ReadOrigin; 1]>;

/// For validation: a list of read origins (previous transaction versions)
/// for each read memory location.
pub type ReadSet = HashMap<MemoryLocationHash, ReadOrigins, BuildIdentityHasher>;

/// The updates made by this transaction incarnation, which is applied
/// to the multi-version data structure at the end of execution.
pub type WriteSet = HashMap<MemoryLocationHash, MemoryValue, BuildIdentityHasher>;
