use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

pub use alloy_primitives::{Signature, SignatureError, Signed, Uint};
pub use hashbrown::HashMap;
pub use revm::bytecode::{
    Bytecode,
    eip7702::Eip7702Bytecode,
    eof::{CodeInfo as EofCodeInfo, EOF_MAGIC_BYTES, EOF_MAGIC_HASH, Eof, EofBody},
    opcode::{OpCode, OpCodeInfo},
};
pub use revm::context::{Block, BlockEnv, CfgEnv, TransactTo, Transaction, TxEnv};
pub use revm::context_interface::{
    block::{BlobExcessGasAndPrice, calc_blob_gasprice, calc_excess_blob_gas},
    cfg::Cfg,
    context::{SStoreResult, SelfDestructResult},
    journaled_state::{AccountLoad, JournalTr, StateLoad},
    result::{
        ExecutionResult, HaltReason, InvalidHeader, InvalidTransaction, OutOfGasError, Output,
        ResultAndState, SuccessReason,
    },
    transaction::{
        AccessList, AccessListItem, Authorization, AuthorizationTr, RecoveredAuthority,
        RecoveredAuthorization, SignedAuthorization, TransactionType,
    },
};
pub use revm::database::{DBErrorMarker, Database, DatabaseCommit, DatabaseRef};
pub use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileSpecId, Precompiles};
pub use revm::primitives::{
    Address, B256, BLOCK_HASH_HISTORY, Bytes, FixedBytes, I256, KECCAK_EMPTY, Log, LogData, TxKind,
    U256, address, alloy_primitives, b256,
    eip7702::{self, PER_AUTH_BASE_COST, PER_EMPTY_ACCOUNT_COST},
    fixed_bytes,
    hardfork::SpecId,
    hex,
    hex::{FromHex, ToHexExt},
    keccak256, uint,
};
pub use revm::state::{
    Account, AccountInfo, AccountStatus, EvmState, EvmStorageSlot as StorageSlot,
};
pub use rustc_hash::FxBuildHasher;

/// Mapping from address to [`Account`].
pub type AccountState = HashMap<Address, Account, BuildSuffixHasher>;

/// Mapping from code hashes to [`Bytecode`]s.
pub type Bytecodes = HashMap<B256, Bytecode, BuildSuffixHasher>;

/// Mapping from block numbers to block hashes.
pub type BlockHashes = HashMap<u64, B256, BuildIdentityHasher>;

/// Use the last 8 bytes of an existing hash like address
/// or code hash instead of rehashing it.
#[derive(Debug, Default)]
pub struct SuffixHasher(u64);

impl Hasher for SuffixHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut suffix = [0u8; 8];
        suffix.copy_from_slice(&bytes[bytes.len() - 8..]);
        self.0 = u64::from_be_bytes(suffix);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// Build a suffix hasher
pub type BuildSuffixHasher = BuildHasherDefault<SuffixHasher>;

/// Converts a [U256] value to a [u64], saturating to [MAX][u64] if the value is too large.
#[macro_export]
macro_rules! as_u64_saturated {
    ($v:expr) => {
        match $v.as_limbs() {
            x => {
                if (x[1] == 0) & (x[2] == 0) & (x[3] == 0) {
                    x[0]
                } else {
                    u64::MAX
                }
            }
        }
    };
}

/// This is primarily used for memory location hash, but can also be used for
/// transaction indexes, etc.
#[derive(Debug, Default)]
pub struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn write_u64(&mut self, id: u64) {
        self.0 = id;
    }
    fn write_usize(&mut self, id: usize) {
        self.0 = id as u64;
    }
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!()
    }
}

/// Build an identity hasher
pub type BuildIdentityHasher = BuildHasherDefault<IdentityHasher>;

/// Calculates the hash of a single value.
#[inline(always)]
pub fn hash_deterministic<T: Hash>(x: T) -> u64 {
    FxBuildHasher.hash_one(x)
}

/// Converts a [U256] value to a [usize], saturating to [MAX][usize] if the value is too large.
#[macro_export]
macro_rules! as_usize_saturated {
    ($v:expr) => {
        usize::try_from($crate::as_u64_saturated!($v)).unwrap_or(usize::MAX)
    };
}

/// Converts a [U256] value to a [isize], saturating to [MAX][isize] if the value is too large.
#[macro_export]
macro_rules! as_isize_saturated {
    ($v:expr) => {
        isize::try_from($crate::as_u64_saturated!($v)).unwrap_or(isize::MAX)
    };
}

/// `const` Option `?`.
#[macro_export]
macro_rules! tri {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return None,
        }
    };
}
