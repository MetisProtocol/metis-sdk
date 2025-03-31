pub use revm::primitives::{
    AccessList, AccessListItem, Address, Authorization, AuthorizationList, B256,
    BLOCK_HASH_HISTORY, BLOCKHASH_SERVE_WINDOW, BLOCKHASH_STORAGE_ADDRESS, BlobExcessGasAndPrice,
    BlockEnv, Bytecode as EVMBytecode, Bytes, CfgEnv, EOF_MAGIC_BYTES, EOF_MAGIC_HASH, Env,
    EvmStorageSlot, FixedBytes, GAS_PER_BLOB, I256, InvalidHeader, InvalidTransaction,
    KECCAK_EMPTY, Log, LogData, MAX_CODE_SIZE, MAX_INITCODE_SIZE,
    RecoveredAuthorization, SignedAuthorization, SpecId, TxEnv, TxKind, TxType, U256, address,
    alloy_primitives, b256, calc_blob_gasprice, calc_excess_blob_gas,
    eip7702::{self, PER_AUTH_BASE_COST, PER_EMPTY_ACCOUNT_COST},
    eof::{Eof, EofBody, TypesSection},
    fixed_bytes, hex,
    hex::{FromHex, ToHexExt},
    keccak256, uint,
};

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
