use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

pub use alloy_evm::EvmEnv;
pub use alloy_primitives::{
    Signature, SignatureError, Signed, TxHash, TxIndex, TxKind, TxNonce, TxNumber, Uint,
    map::DefaultHashBuilder,
};
pub use revm::bytecode::{
    Bytecode,
    eip7702::Eip7702Bytecode,
    opcode::{OpCode, OpCodeInfo},
};
pub use revm::context::{
    Block, BlockEnv, CfgEnv, ContextError, ContextSetters, ContextTr, TransactTo, Transaction,
    TxEnv, result::EVMError,
};
pub use revm::context_interface::{
    block::{BlobExcessGasAndPrice, calc_blob_gasprice},
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
pub use revm::database::{
    Cache, CacheDB, CacheState, DBErrorMarker, Database, DatabaseCommit, DatabaseRef, EmptyDB,
    PlainAccount, State,
};
pub use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileSpecId, Precompiles};
pub use revm::primitives::{
    Address, B256, BLOCK_HASH_HISTORY, Bytes, FixedBytes, HashMap, I256, KECCAK_EMPTY, Log,
    LogData, U256, address, alloy_primitives, b256,
    eip4844::{self, GAS_PER_BLOB},
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
pub use revm::{Context, MainBuilder, MainContext, MainnetEvm};
pub use revm::{ExecuteCommitEvm, ExecuteEvm, InspectEvm};
pub use rustc_hash::FxBuildHasher;

pub mod i257;
pub use i257::I257;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub block: BlockEnv,
    pub tx: TxEnv,
    pub cfg: CfgEnv,
}

/// Mapping from address to [`Account`].
pub type AccountState = HashMap<Address, Account>;

/// Mapping from code hashes to [`Bytecode`]s.
pub type Bytecodes = HashMap<B256, Bytecode>;

/// Mapping from block numbers to block hashes.
pub type BlockHashes = HashMap<u64, B256>;

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
