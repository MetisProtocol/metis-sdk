use metis_pe::{EvmAccount, ParallelExecutor, chain::Chain};
use revm::DatabaseRef;
use revm::context::{BlockEnv, TxEnv};
use revm::primitives::hardfork::SpecId;
use revm::primitives::{Address, U256, alloy_primitives::U160};
use std::{num::NonZeroUsize, thread};

/// Mock an account from an integer index that is used as the address.
/// Useful for mock iterations.
pub fn mock_account(idx: usize) -> (Address, EvmAccount) {
    let address = Address::from(U160::from(idx));
    let account = EvmAccount {
        // Filling half full accounts to have enough tokens for tests without worrying about
        // the corner case of balance not going beyond [U256::MAX].
        balance: U256::MAX.div_ceil(U256::from(2)),
        nonce: 1,
        ..EvmAccount::default()
    };
    (address, account)
}

/// Execute an REVM block sequentially and parallelly with PEVM and assert that
/// the execution results match.
pub fn test_execute_revm<C, S>(chain: &C, storage: S, txs: Vec<TxEnv>)
where
    C: Chain + PartialEq + Send + Sync,
    S: DatabaseRef + Send + Sync,
{
    let concurrency_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let mut pe = ParallelExecutor::default();
    assert_eq!(
        metis_pe::execute_revm_sequential(
            chain,
            &storage,
            SpecId::PRAGUE,
            BlockEnv::default(),
            txs.clone(),
            #[cfg(feature = "compiler")]
            pe.worker.clone(),
        ),
        pe.execute_revm_parallel(
            chain,
            &storage,
            SpecId::PRAGUE,
            BlockEnv::default(),
            txs,
            concurrency_level,
        ),
    );
}
