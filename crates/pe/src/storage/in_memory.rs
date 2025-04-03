use std::fmt::Debug;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256, keccak256};
use metis_primitives::EVMBytecode;
use revm::Database;
use super::{BlockHashes, Bytecodes, ChainState, StorageError};
use crate::{AccountBasic};
use crate::storage::Storage;
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

impl Storage for InMemoryStorage {
    fn code_hash(&self, address: &Address) -> Result<Option<B256>, StorageError> {
        Ok(self
            .accounts
            .get(address)
            .and_then(|account| account.code_hash))
    }
}

impl Database for InMemoryStorage {
    // TODO: More proper error handling
    type Error = StorageError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountBasic>, Self::Error> {
        Ok(self.accounts.get(&address).map(|account| AccountBasic {
            balance: account.balance,
            nonce: account.nonce,
        }))
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Option<EVMBytecode>, Self::Error> {
        Ok(self.bytecodes.get(&code_hash).cloned())
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
            // Matching REVM's [EmptyDB] for now
            .unwrap_or_else(|| keccak256(number.to_string().as_bytes())))
    }
}
