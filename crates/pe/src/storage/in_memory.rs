use std::fmt::Debug;
use std::sync::Arc;

use super::StorageError;
use metis_primitives::{
    Address, B256, BlockHashes, Bytecode, Bytecodes, ChainState, U256, keccak256,
};
use revm::state::AccountInfo;
use revm::{Database, DatabaseRef};

/// A storage that stores chain data in memory.
#[derive(Debug, Clone, Default)]
pub struct InMemoryStorage {
    accounts: ChainState,
    bytecodes: Arc<Bytecodes>,
    block_hashes: Arc<BlockHashes>,
}

impl InMemoryStorage {
    /// Construct a new [`InMemoryStorage`]
    pub const fn new(
        accounts: ChainState,
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

impl Database for InMemoryStorage {
    type Error = StorageError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.accounts.get(&address).map(|account| AccountInfo {
            balance: account.balance,
            nonce: account.nonce,
            code_hash: account.code_hash,
            code: account.code.clone(),
        }))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.bytecodes
            .get(&code_hash)
            .cloned()
            .ok_or_else(|| StorageError::StorageNotFound("code_hash_not_found".to_owned()))
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|account| account.storage.get(&index))
            .copied()
            .unwrap_or_default())
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(self
            .block_hashes
            .get(&number)
            .copied()
            .unwrap_or_else(|| keccak256(number.to_string().as_bytes())))
    }
}

impl DatabaseRef for InMemoryStorage {
    type Error = StorageError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.accounts.get(&address).map(|account| AccountInfo {
            balance: account.balance,
            nonce: account.nonce,
            code_hash: account.code_hash,
            code: account.code.clone(),
        }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.bytecodes
            .get(&code_hash)
            .cloned()
            .ok_or_else(|| StorageError::StorageNotFound("code_hash_not_found".to_owned()))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|account| account.storage.get(&index))
            .copied()
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
