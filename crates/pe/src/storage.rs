use std::fmt::{Debug, Display};

use alloy_primitives::{Address, B256, U256};
use hashbrown::HashMap;
use revm::{
    DatabaseRef,
    bytecode::Bytecode,
    context::DBErrorMarker,
    primitives::KECCAK_EMPTY,
    state::{Account, AccountInfo},
};
use rustc_hash::FxBuildHasher;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BuildIdentityHasher, BuildSuffixHasher};

// TODO: Port EVM types to [primitives.rs] to focus solely
// on the [Storage] interface here.

/// An EVM account.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmAccount {
    /// The account's balance.
    pub balance: U256,
    /// The account's nonce.
    pub nonce: u64,
    /// The optional code hash of the account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_hash: Option<B256>,
    /// The account's optional code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Bytecode>,
    /// The account's storage.
    pub storage: HashMap<U256, U256, FxBuildHasher>,
}

impl From<Account> for EvmAccount {
    fn from(account: Account) -> Self {
        let has_code = !account.info.is_empty_code_hash();
        Self {
            balance: account.info.balance,
            nonce: account.info.nonce,
            code_hash: has_code.then_some(account.info.code_hash),
            code: has_code.then(|| account.info.code.unwrap_or_default()),
            storage: account
                .storage
                .into_iter()
                .map(|(k, v)| (k, v.present_value))
                .collect(),
        }
    }
}

/// Basic information of an account
// TODO: Reuse something sane from Alloy?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBasic {
    /// The balance of the account.
    pub balance: U256,
    /// The nonce of the account.
    pub nonce: u64,
}

impl Default for AccountBasic {
    fn default() -> Self {
        Self {
            balance: U256::ZERO,
            nonce: 0,
        }
    }
}

/// Mapping from address to [`EvmAccount`]
pub type ChainState = HashMap<Address, EvmAccount, BuildSuffixHasher>;

/// Mapping from code hashes to [`Bytecode`]s
pub type Bytecodes = HashMap<B256, Bytecode, BuildSuffixHasher>;

/// Mapping from block numbers to block hashes
pub type BlockHashes = HashMap<u64, B256, BuildIdentityHasher>;

/// An interface to provide chain state for the transaction execution.
/// Staying close to the underlying REVM's Database trait while not leaking
/// its primitives to library users (favoring Alloy at the moment).
/// TODO: Better API for third-party integration.
pub trait Storage {
    /// Errors when querying data from storage.
    type Error: Display + Debug;

    /// Get basic account information.
    fn basic(&self, address: &Address) -> Result<Option<AccountBasic>, Self::Error>;

    /// Get the code of an account.
    fn code_hash(&self, address: &Address) -> Result<Option<B256>, Self::Error>;

    /// Get account code by its hash.
    fn code_by_hash(&self, code_hash: &B256) -> Result<Option<Bytecode>, Self::Error>;

    /// Get storage value of address at index.
    fn storage(&self, address: &Address, index: &U256) -> Result<U256, Self::Error>;

    /// Get block hash by block number.
    fn block_hash(&self, number: &u64) -> Result<B256, Self::Error>;
}

/// revm [Database] errors when using [Storage] as the underlying provider.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum StorageWrapperError<S: Storage> {
    #[error("storage error")]
    StorageError(S::Error),
}

impl<S: Storage> DBErrorMarker for StorageWrapperError<S> {}

/// A Storage wrapper that implements REVM's [`DatabaseRef`] for ease of
/// integration.
#[derive(Debug)]
pub struct StorageWrapper<'a, S: Storage>(pub &'a S);

impl<S: Storage + Debug> DatabaseRef for StorageWrapper<'_, S> {
    type Error = StorageWrapperError<S>;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let Some(basic) = self
            .0
            .basic(&address)
            .map_err(StorageWrapperError::StorageError)?
        else {
            return Ok(None);
        };

        let code_hash = self
            .0
            .code_hash(&address)
            .map_err(StorageWrapperError::StorageError)?;

        let code = if let Some(hash) = &code_hash {
            self.0
                .code_by_hash(hash)
                .map_err(StorageWrapperError::StorageError)?
        } else {
            None
        };

        Ok(Some(AccountInfo {
            balance: basic.balance,
            nonce: basic.nonce,
            code_hash: code_hash.unwrap_or(KECCAK_EMPTY),
            code,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self
            .0
            .code_by_hash(&code_hash)
            .map_err(StorageWrapperError::StorageError)?
            .unwrap_or_default())
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.0
            .storage(&address, &index)
            .map_err(StorageWrapperError::StorageError)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.0
            .block_hash(&number)
            .map_err(StorageWrapperError::StorageError)
    }
}

mod in_memory;
pub use in_memory::InMemoryStorage;
#[cfg(feature = "rpc-storage")]
mod rpc;
#[cfg(feature = "rpc-storage")]
pub use rpc::RpcStorage;