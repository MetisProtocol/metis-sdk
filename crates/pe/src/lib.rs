//! Blazingly fast Parallel EVM in Rust.

// TODO: Better types & API for third-party integration
use bitflags::bitflags;
use rustc_hash::FxBuildHasher;
use std::hash::{BuildHasher, Hash};
// This optimization is desired as we constantly index into many
// vectors of the block-size size. It can yield up to 5% improvement.
macro_rules! index_mutex {
    ($vec:expr, $index:expr) => {
        // SAFETY: A correct scheduler would not leak indexes larger
        // than the block size, which is the size of all vectors we
        // index via this macro. Otherwise, DO NOT USE!
        // TODO: Better error handling for the mutex.
        unsafe { $vec.get_unchecked($index).lock().unwrap() }
    };
}

// TODO: Ensure it's not easy to hand-craft transactions and storage slots
// that can cause a lot of collisions that destroys pe's performance.
#[inline(always)]
fn hash_deterministic<T: Hash>(x: T) -> u64 {
    FxBuildHasher.hash_one(x)
}

bitflags! {
    struct FinishExecFlags: u8 {
        // Do we need to validate from this transaction?
        // The first and lazy transactions don't need validation. Note
        // that this is used to tune the min validation index in the
        // scheduler, meaning a [false] here will still be validated if
        // there was a lower transaction that has broken the preprocessed
        // dependency chain and returned [true]
        const NeedValidation = 0;
        // We need to validate from the next transaction if this execution
        // wrote to a new location.
        const WroteNewLocation = 1;
    }
}

pub mod chain;
pub mod compat;
mod executor;
mod mv_memory;
pub mod schedulers;
pub mod types;
pub use executor::{
    ParallelExecutor, ParallelExecutorError, ParallelExecutorResult, execute_revm_sequential,
};
pub use schedulers::{DAGProvider, NormalProvider};
pub use types::*;
mod storage;
pub use storage::{
    AccountBasic, BlockHashes, Bytecodes, ChainState, EvmAccount, InMemoryStorage, StorageError,
};
mod vm;
pub use vm::{ExecutionError, TxExecutionResult};
