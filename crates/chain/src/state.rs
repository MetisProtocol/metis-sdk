use std::fmt::Debug;
use reth::{
    revm::{
        db::{State},
    },
};
use revm::Database;
use alloy_primitives::{Address, B256, U256};
use reth::revm::bytecode::Bytecode;
use reth::revm::state::AccountInfo;
use metis_pe::Storage;
use metis_pe::storage::StorageError;

#[derive(Debug)]
pub struct StateStorageAdapter<'a, DB>(pub &'a mut State<DB>);

impl<'a, DB> Database for StateStorageAdapter<'a, DB>
where
    DB: Database + Debug + 'static,
    DB::Error: Into<StorageError> + Send + Sync + Debug + 'static,
{
    type Error = StorageError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.0.basic(address).map_err(Into::into)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.0.code_by_hash(code_hash).map_err(Into::into)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.0.storage(address, index).map_err(Into::into)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.0.block_hash(number).map_err(Into::into)
    }
}

impl<'a, DB> Storage for StateStorageAdapter<'a, DB>
where
    DB: Database + Send + Sync + 'static,
    DB::Error: Into<StorageError> + Send + Sync,
{
    fn code_hash(&mut self, address: &Address) -> Result<Option<B256>, StorageError> {
        let account_info = self.0.basic(*address).map_err(Into::into)?;
        Ok(account_info.map(|info| info.code_hash))
    }
}