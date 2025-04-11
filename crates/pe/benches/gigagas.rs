use alloy_evm::EvmEnv;
use alloy_primitives::{Address, U160, U256};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use metis_pe::{
    AccountState, Bytecodes, EvmAccount, InMemoryDB, ParallelExecutor, execute_revm_sequential,
};
use revm::context::{TransactTo, TxEnv};
use std::{num::NonZeroUsize, sync::Arc, thread};

#[path = "../tests/common/mod.rs"]
pub mod common;
#[path = "../tests/erc20/mod.rs"]
pub mod erc20;
#[path = "../tests/uniswap/mod.rs"]
pub mod uniswap;

const GIGA_GAS: u64 = 1_000_000_000;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Runs a benchmark for executing a set of transactions on a given blockchain state.
pub fn bench(c: &mut Criterion, name: &str, db: InMemoryDB, txs: Vec<TxEnv>) {
    let concurrency_level = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let mut pe = ParallelExecutor::default();
    let mut group = c.benchmark_group(name);
    group.bench_function("Sequential", |b| {
        b.iter(|| {
            execute_revm_sequential(
                black_box(&db),
                black_box(EvmEnv::default()),
                black_box(txs.clone()),
                #[cfg(feature = "compiler")]
                pe.worker.clone(),
            )
        })
    });
    group.bench_function("Parallel", |b| {
        b.iter(|| {
            pe.execute_revm_parallel(
                black_box(&db),
                black_box(EvmEnv::default()),
                black_box(txs.clone()),
                black_box(concurrency_level),
            )
        })
    });
    #[cfg(feature = "compiler")]
    {
        let mut pe = ParallelExecutor::compiler();
        group.bench_function("Sequential-With-Compiler", |b| {
            b.iter(|| {
                execute_revm_sequential(
                    black_box(&db),
                    black_box(EvmEnv::default()),
                    black_box(txs.clone()),
                    pe.worker.clone(),
                )
            })
        });
        group.bench_function("Parallel-With-Compiler", |b| {
            b.iter(|| {
                pe.execute_revm_parallel(
                    black_box(&db),
                    black_box(EvmEnv::default()),
                    black_box(txs.clone()),
                    black_box(concurrency_level),
                )
            })
        });
    }
    group.finish();
}

/// Benchmarks the execution time of raw token transfers.
pub fn bench_raw_transfers(c: &mut Criterion) {
    let block_size = (GIGA_GAS as f64 / common::RAW_TRANSFER_GAS_LIMIT as f64).ceil() as usize;
    // Skip the built-in precompiled contracts addresses.
    const START_ADDRESS: usize = 1000;
    const MINER_ADDRESS: usize = 0;
    let db = InMemoryDB::new(
        std::iter::once(MINER_ADDRESS)
            .chain(START_ADDRESS..START_ADDRESS + block_size)
            .map(common::mock_account)
            .collect(),
        Default::default(),
        Default::default(),
    );
    bench(
        c,
        "Independent Raw Transfers",
        db,
        (0..block_size)
            .map(|i| {
                let address = Address::from(U160::from(START_ADDRESS + i));
                TxEnv {
                    caller: address,
                    kind: TransactTo::Call(address),
                    value: U256::from(1),
                    gas_limit: common::RAW_TRANSFER_GAS_LIMIT,
                    gas_price: 1_u128,
                    ..TxEnv::default()
                }
            })
            .collect::<Vec<_>>(),
    );
}

/// Benchmarks the execution time of ERC-20 token transfers.
pub fn bench_erc20(c: &mut Criterion) {
    let block_size = (GIGA_GAS as f64 / erc20::ESTIMATED_GAS_USED as f64).ceil() as usize;
    let (mut state, bytecodes, txs) = erc20::generate_cluster(block_size, 1, 1);
    state.insert(Address::ZERO, EvmAccount::default()); // Beneficiary
    bench(
        c,
        "Independent ERC20",
        InMemoryDB::new(state, Arc::new(bytecodes), Default::default()),
        txs,
    );
}

/// Benchmarks the execution time of Uniswap V3 swap transactions.
pub fn bench_uniswap(c: &mut Criterion) {
    let block_size = (GIGA_GAS as f64 / uniswap::ESTIMATED_GAS_USED as f64).ceil() as usize;
    let mut final_state = AccountState::from_iter([(Address::ZERO, EvmAccount::default())]); // Beneficiary
    let mut final_bytecodes = Bytecodes::default();
    let mut final_txs = Vec::<TxEnv>::new();
    for _ in 0..block_size {
        let (state, bytecodes, txs) = uniswap::generate_cluster(1, 1);
        final_state.extend(state);
        final_bytecodes.extend(bytecodes);
        final_txs.extend(txs);
    }
    bench(
        c,
        "Independent Uniswap",
        InMemoryDB::new(final_state, Arc::new(final_bytecodes), Default::default()),
        final_txs,
    );
}

/// Runs a series of benchmarks to evaluate the performance of different transaction types.
pub fn benchmark_gigagas(c: &mut Criterion) {
    bench_raw_transfers(c);
    bench_erc20(c);
    bench_uniswap(c);
}

// HACK: we can't document public items inside of the macro
#[allow(missing_docs)]
mod benches {
    use super::*;
    // criterion_group!(benches, benchmark_gigagas);
    criterion_group! {
        name = benches;
        config = Criterion::default().sample_size(10).measurement_time(std::time::Duration::from_secs(6));
        targets = benchmark_gigagas
    }
}

criterion_main!(benches::benches);
