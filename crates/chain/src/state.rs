use reth::{
    api::{ConfigureEvm},
    revm::{
        db::{State},
    },
};
use reth_evm::{
    Evm,
    execute::{BlockExecutionError, BlockExecutor, BlockExecutorProvider, Executor},
};
use std::sync::{Arc, Mutex};
use alloy_primitives::{Address, B256, U256};
use reth::revm::Database;
use metis_pe::{AccountBasic, Storage};
use revm::bytecode::Bytecode;

#[derive(Clone)]
pub struct StateStorageAdapter<DB> {
    pub(crate) state: Mutex<State<DB>>,
}

impl<DB> StateStorageAdapter<DB> {
    pub fn new(state: State<DB>) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }
}

impl<DB: Database + Send + Sync + 'static> Storage for StateStorageAdapter<DB> {
    type Error = BlockExecutionError;

    fn basic(&self, address: &Address) -> Result<Option<AccountBasic>, Self::Error> {
        let account = self.state.lock().unwrap().basic(*address)?;
        Ok(account.map(|account| AccountBasic {
            balance: account.balance,
            nonce: account.nonce,
        }))
    }

    fn code_hash(&self, address: &Address) -> Result<Option<B256>, Self::Error> {
        Ok(self
            .state.lock().unwrap().cache.accounts
            .get(address)
            .and_then(|cache| {
                let acc =  cache.clone().account;
                acc.map(|x| x.info.code_hash)
            })
        )
    }

    fn code_by_hash(&self, code_hash: &B256) -> Result<Option<Bytecode>, Self::Error> {
        self.state.lock().unwrap().code_by_hash(*code_hash)
    }

    fn storage(&self, address: &Address, index: &U256) -> Result<U256, Self::Error> {
        self.state.lock().unwrap().storage(*address, *index)
    }

    fn block_hash(&self, number: &u64) -> Result<B256, Self::Error> {
        self.state.lock().unwrap().block_hash(*number)
    }
}