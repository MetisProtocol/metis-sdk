use std::fmt::Debug;

use metis_primitives::{Address, DBErrorMarker};

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
