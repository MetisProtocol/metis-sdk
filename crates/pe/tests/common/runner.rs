use alloy_evm::EvmEnv;
use metis_pe::{Account, AccountInfo, ParallelExecutor};
use pretty_assertions::assert_eq;
use revm::DatabaseRef;
use revm::context::TxEnv;
use revm::primitives::{Address, U256, alloy_primitives::U160};
use std::{num::NonZeroUsize, thread};

/// Mock an account from an integer index that is used as the address.
/// Useful for mock iterations.
pub fn mock_account(idx: usize) -> (Address, Account) {
    let address = Address::from(U160::from(idx));
    let account = Account {
        info: AccountInfo {
            // Filling half full accounts to have enough tokens for tests without worrying about
            // the corner case of balance not going beyond [U256::MAX].
            balance: U256::MAX.div_ceil(U256::from(2)),
            nonce: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    (address, account)
}

/// Execute an REVM block sequentially and parallelly with PEVM and assert that
/// the execution results match.
pub fn test_execute<DB>(db: DB, txs: Vec<TxEnv>)
where
    DB: DatabaseRef + Send + Sync,
{
    let concurrency_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let mut pe: ParallelExecutor<metis_primitives::HaltReason> = ParallelExecutor::default();

    // Compare receipts only: `TxExecutionResult` contains `ResultAndState`, which is not
    // guaranteed to implement `PartialEq` across upstream versions.
    let seq_receipts = metis_pe::execute_sequential::<_, metis_primitives::HaltReason>(
        &db,
        EvmEnv::default(),
        txs.clone(),
        #[cfg(feature = "compiler")]
        pe.worker.clone(),
    )
    .map(|results| results.into_iter().map(|r| r.receipt).collect::<Vec<_>>());

    let par_receipts = pe
        .execute(&db, EvmEnv::default(), txs, concurrency_level)
        .map(|results| results.into_iter().map(|r| r.receipt).collect::<Vec<_>>());

    assert_eq!(seq_receipts, par_receipts);
}
