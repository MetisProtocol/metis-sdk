use std::{fmt::Display, sync::Arc};

use alloy_primitives::{Address, B256, Bytes, U256};
use hashbrown::HashMap;
use revm::{
    DatabaseRef,
    interpreter::analysis::to_analysed,
    primitives::{
        Account, AccountInfo, Bytecode, EIP7702_MAGIC_BYTES, Eip7702Bytecode, Eof, KECCAK_EMPTY,
        LegacyAnalyzedBytecode,
    },
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
    pub code: Option<EvmCode>,
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
            code: has_code.then(|| account.info.code.unwrap().into()),
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

/// EIP7702 delegated code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eip7702Code {
    /// Address of the EOA which will inherit the bytecode.
    delegated_address: Address,
    /// Version of the bytecode.
    version: u8,
}

/// EVM Code, currently mapping to REVM's [`ByteCode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvmCode {
    /// Maps both analyzed and non-analyzed REVM legacy bytecode.
    Legacy(LegacyAnalyzedBytecode),
    /// Maps delegated EIP7702 bytecode.
    Eip7702(Eip7702Code),
    /// Maps EOF bytecode.
    Eof(Bytes),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeConversionError {
    #[error("Failed to decode EOF")]
    EofDecodingError(#[source] revm::primitives::eof::EofDecodeError),
}

impl TryFrom<EvmCode> for Bytecode {
    type Error = BytecodeConversionError;
    fn try_from(code: EvmCode) -> Result<Self, Self::Error> {
        match code {
            // TODO: Turn this [unsafe] into a proper [Result]
            EvmCode::Legacy(code) => Ok(Self::LegacyAnalyzed(code)),
            EvmCode::Eip7702(code) => {
                let mut raw = EIP7702_MAGIC_BYTES.to_vec();
                raw.push(code.version);
                raw.extend(&code.delegated_address);
                Ok(Self::Eip7702(Eip7702Bytecode {
                    delegated_address: code.delegated_address,
                    version: code.version,
                    raw: raw.into(),
                }))
            }
            EvmCode::Eof(code) => Eof::decode(code)
                .map(Arc::new)
                .map(Self::Eof)
                .map_err(Self::Error::EofDecodingError),
        }
    }
}

impl From<Bytecode> for EvmCode {
    fn from(code: Bytecode) -> Self {
        match code {
            // This arm will recursively fallback to LegacyAnalyzed.
            Bytecode::LegacyRaw(_) => to_analysed(code).into(),
            Bytecode::LegacyAnalyzed(code) => Self::Legacy(code),
            Bytecode::Eip7702(code) => Self::Eip7702(Eip7702Code {
                delegated_address: code.delegated_address,
                version: code.version,
            }),
            Bytecode::Eof(code) => Self::Eof(Arc::unwrap_or_clone(code).raw),
        }
    }
}

/// Mapping from address to [`EvmAccount`]
pub type ChainState = HashMap<Address, EvmAccount, BuildSuffixHasher>;

/// Mapping from code hashes to [`EvmCode`]s
pub type Bytecodes = HashMap<B256, EvmCode, BuildSuffixHasher>;

/// Mapping from block numbers to block hashes
pub type BlockHashes = HashMap<u64, B256, BuildIdentityHasher>;

/// An interface to provide chain state for the transaction execution.
/// Staying close to the underlying REVM's Database trait while not leaking
/// its primitives to library users (favoring Alloy at the moment).
/// TODO: Better API for third-party integration.
pub trait Storage {
    /// Errors when querying data from storage.
    type Error: Display;

    /// Get basic account information.
    fn basic(&self, address: &Address) -> Result<Option<AccountBasic>, Self::Error>;

    /// Get the code of an account.
    fn code_hash(&self, address: &Address) -> Result<Option<B256>, Self::Error>;

    /// Get account code by its hash.
    fn code_by_hash(&self, code_hash: &B256) -> Result<Option<EvmCode>, Self::Error>;

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
    #[error("invalid byte code")]
    InvalidBytecode(BytecodeConversionError),
}

/// A Storage wrapper that implements REVM's [`DatabaseRef`] for ease of
/// integration.
#[derive(Debug)]
pub struct StorageWrapper<'a, S: Storage>(pub &'a S);

impl<S: Storage> DatabaseRef for StorageWrapper<'_, S> {
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
                .map(|c| Bytecode::try_from(c).map_err(StorageWrapperError::InvalidBytecode))
                .transpose()?
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
        self.0
            .code_by_hash(&code_hash)
            .map_err(StorageWrapperError::StorageError)
            .and_then(|evm_code| {
                evm_code
                    .map(Bytecode::try_from)
                    .transpose()
                    .map(|bytecode: Option<Bytecode>| bytecode.unwrap_or_default())
                    .map_err(StorageWrapperError::InvalidBytecode)
            })
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
