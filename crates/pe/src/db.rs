use metis_primitives::{
    AccountState, Address, B256, BlockHashes, Bytecode, Bytecodes, DBErrorMarker, KECCAK_EMPTY,
    U256, keccak256,
};
use revm::DatabaseRef;
use revm::state::AccountInfo;
use std::fmt::Debug;
use std::sync::Arc;

/// Memory Database error definitions
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DBError {
    #[error("Storage error: {0}")]
    StorageNotFound(String),
}

impl DBErrorMarker for DBError {}

/// A memory DB that stores the account and bytecode data in memory.
/// Just for testing, in the real chain, we used the reth database.
#[derive(Debug, Clone, Default)]
pub struct InMemoryDB {
    accounts: AccountState,
    bytecodes: Arc<Bytecodes>,
    block_hashes: Arc<BlockHashes>,
}

impl InMemoryDB {
    /// Construct a new [`InMemoryDB`]
    pub const fn new(
        accounts: AccountState,
        bytecodes: Arc<Bytecodes>,
        block_hashes: Arc<BlockHashes>,
    ) -> Self {
        Self {
            accounts,
            bytecodes,
            block_hashes,
        }
    }
}

impl DatabaseRef for InMemoryDB {
    type Error = DBError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self
            .accounts
            .get(&address)
            .map(|account| account.info.clone()))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == KECCAK_EMPTY {
            Ok(Bytecode::default())
        } else {
            self.bytecodes.get(&code_hash).cloned().ok_or_else(|| {
                DBError::StorageNotFound(format!("code_hash_not_found {}", code_hash))
            })
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|account| account.storage.get(&index).map(|v| v.present_value))
            .unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        Ok(self
            .block_hashes
            .get(&number)
            .copied()
            .unwrap_or_else(|| keccak256(number.to_string().as_bytes())))
    }
}
