use alloy_primitives::{Address, B256, U256};
use reth::revm::db::State;
use revm::bytecode::Bytecode;
use revm::state::AccountInfo;
use revm::{Database, DatabaseRef};
use std::sync::RwLock;

pub struct StateStorageAdapter<'a, DB> {
    pub(crate) state: RwLock<&'a mut State<DB>>,
}

unsafe impl<DB> Send for StateStorageAdapter<'_, DB> {}
unsafe impl<DB> Sync for StateStorageAdapter<'_, DB> {}

impl<'a, DB> StateStorageAdapter<'a, DB> {
    pub fn new(state: &'a mut State<DB>) -> Self {
        Self {
            state: RwLock::new(state),
        }
    }
}

impl<DB: Database> DatabaseRef for StateStorageAdapter<'_, DB> {
    type Error = DB::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.state.write().unwrap().basic(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.state.write().unwrap().code_by_hash(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.state.write().unwrap().storage(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.state.write().unwrap().block_hash(number)
    }
}
