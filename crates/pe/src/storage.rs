use alloy_primitives::{Address, B256, U256};
use hashbrown::HashMap;
use revm::{bytecode::Bytecode, context::DBErrorMarker, state::Account};
use rustc_hash::FxBuildHasher;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

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

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("In-memory storage error: {0}")]
    Memory(u8),

    #[error("Account not found: {0:?}")]
    AccountNotFound(Address),

    #[error("Storage error: {0}")]
    StorageNotFound(String),
}

impl From<u8> for StorageError {
    fn from(code: u8) -> Self {
        StorageError::Memory(code)
    }
}

impl DBErrorMarker for StorageError {}

mod in_memory;
pub use in_memory::InMemoryStorage;
