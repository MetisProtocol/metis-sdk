use alloy_evm::EvmEnv;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressDrawTarget};
use metis_chain::state::StateStorageAdapter;
use metis_pe::{AccountInfo, ParallelExecutor, TxExecutionResult, execute_sequential};
use metis_primitives::{
    AccessListItem, Address, B256, Bytecode, Bytes, HashMap, SpecId, TxEnv, U256, as_u64_saturated,
    keccak256,
};
use metis_tools::{find_all_json_tests, get_block_spec};
use pretty_assertions::assert_eq;
use revm::database::{CacheState, State};
use revm::primitives::TxKind;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Parser)]
#[command(author, version, about)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run block tests with given parameters
    Run(RunArgs),
}

#[derive(Args)]
struct RunArgs {
    path: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    env: TestEnv,
    // The key denotes the tx index + 1
    transactions: HashMap<String, TestTransaction>,
    // The key denotes the address
    pre: HashMap<Address, Account>,
    // The key denotes the address
    post: Option<HashMap<Address, Account>>,
    // The key denotes the tx index
    logs: HashMap<String, HashMap<String, ExpectLog>>,
    /// Predefined block hashes
    block_hashes: Option<HashMap<String, B256>>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
struct TestEnv {
    pub block_number: U256,
    pub current_coinbase: Address,
    pub current_difficulty: U256,
    pub current_gas_limit: U256,
    pub current_timestamp: U256,
    pub previous_hash: B256,
    pub base_fee_per_gas: Option<U256>,
    pub mix_hash: Option<B256>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, Clone)]
struct TestTransaction {
    pub data: Bytes,
    pub gas_limit: U256,
    pub gas_price: Option<U256>,
    pub nonce: U256,
    #[serde(skip)]
    pub secret_key: B256,
    #[serde(default)]
    pub sender: Option<Address>,
    pub to: Option<Address>,
    pub value: U256,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub max_fee_per_blob_gas: Option<U256>,
    pub access_list: Option<Vec<AccessListItem>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub balance: U256,
    pub code: Bytes,
    pub code_hash: Option<B256>,
    pub nonce: u64,
    pub storage: HashMap<U256, U256>,
}

#[derive(Debug, Error)]
#[error("test {name} suite {suite_name:?} failed")]
pub struct TestError {
    pub name: String,
    pub suite_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum TestErrorKind {
    #[error("account state mismatch: got {got:?}, expected {expected:?}")]
    AccountMismatch {
        got: (Address, U256, u64),
        expected: (Address, U256, u64),
    },
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExpectLog {
    pub address: Address,
    pub data: Bytes,
    pub topics: Vec<B256>,
    pub block_hash: Option<B256>,
    pub block_number: Option<U256>,
    pub transaction_hash: Option<B256>,
    pub transaction_index: Option<U256>,
    pub log_index: Option<U256>,
    pub removed: Option<bool>,
}

fn execute_test(path: &Path) -> Result<(), TestError> {
    let s = std::fs::read_to_string(path).unwrap();
    let suites: HashMap<String, Suite> = serde_json::from_str(&s).unwrap();
    let mut cache_state = CacheState::new(false);
    for (name, suite) in &suites {
        for (address, info) in &suite.pre {
            let code_hash = keccak256(info.code.clone());
            if let Some(cc_code_hash) = info.code_hash {
                assert_eq!(cc_code_hash, code_hash);
            }
            let bytecode = Bytecode::new_raw(info.code.clone());
            cache_state.contracts.insert(code_hash, bytecode.clone());
            cache_state.insert_account_with_storage(
                *address,
                AccountInfo {
                    balance: info.balance,
                    code_hash,
                    code: Some(bytecode),
                    nonce: info.nonce,
                },
                info.storage.iter().map(|(k, v)| (*k, *v)).collect(),
            );
        }
        let spec_id = get_block_spec(
            as_u64_saturated!(suite.env.current_timestamp),
            as_u64_saturated!(suite.env.block_number),
        );
        // Sort transactions by id
        let mut transactions = (0..suite.transactions.len())
            .map(|_| TestTransaction::default())
            .collect::<Vec<_>>();
        for (id, tx) in &suite.transactions {
            transactions[id.parse::<usize>().unwrap() - 1] = tx.clone();
        }
        let mut env: EvmEnv<SpecId, revm::context::BlockEnv> = EvmEnv::default();
        env.cfg_env.chain_id = 1;
        env.cfg_env.spec = spec_id;
        env.block_env.number = suite.env.block_number;
        env.block_env.beneficiary = suite.env.current_coinbase;
        env.block_env.gas_limit = as_u64_saturated!(suite.env.current_gas_limit);
        env.block_env.timestamp = suite.env.current_timestamp;
        env.block_env.difficulty = suite.env.current_difficulty;
        env.block_env.prevrandao = Some(suite.env.mix_hash.unwrap_or_default());
        env.block_env.basefee = suite
            .env
            .base_fee_per_gas
            .unwrap_or_default()
            .try_into()
            .unwrap_or(u64::MAX);
        let mut txs = Vec::new();
        for tx in transactions {
            let mut tx = TxEnv {
                data: tx.data.clone(),
                gas_limit: as_u64_saturated!(tx.gas_limit),
                gas_price: tx
                    .gas_price
                    .or(tx.max_fee_per_gas)
                    .unwrap_or_default()
                    .try_into()
                    .unwrap_or(u128::MAX),
                nonce: as_u64_saturated!(tx.nonce),
                caller: tx.sender.unwrap_or_default(),
                value: tx.value,
                kind: match tx.to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                access_list: tx.access_list.clone().unwrap_or_default().into(),
                gas_priority_fee: None,
                ..Default::default()
            };
            let _ = tx.derive_tx_type();
            txs.push(tx);
        }
        // Explicit HR to avoid inference ambiguity after HR was made generic in metis-pe.
        let mut executor: ParallelExecutor<metis_primitives::HaltReason> =
            ParallelExecutor::default();
        // Clone the state for execution.
        let mut cache = cache_state.clone();
        cache.set_state_clear_flag(spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON));
        let mut state = State::builder()
            .with_cached_prestate(cache)
            .with_bundle_update()
            .with_block_hashes(
                suite
                    .block_hashes
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(|(k, v)| (k.parse().unwrap(), *v))
                    .collect(),
            )
            .build();
        // Check sequential execute results
        let sequential_results = execute_sequential::<_, metis_primitives::HaltReason>(
            StateStorageAdapter::new(&mut state),
            env.clone(),
            txs.clone(),
            #[cfg(feature = "compiler")]
            executor.worker.clone(),
        )
        .unwrap();
        check_execute_results(&sequential_results, name, suite);
        // Clone the state for execution.
        let mut cache = cache_state.clone();
        cache.set_state_clear_flag(spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON));
        let mut state = State::builder()
            .with_cached_prestate(cache)
            .with_bundle_update()
            .with_block_hashes(
                suite
                    .block_hashes
                    .clone()
                    .unwrap_or_default()
                    .iter()
                    .map(|(k, v)| (k.parse().unwrap(), *v))
                    .collect(),
            )
            .build();
        // Check parallel execute results
        let concurrency_level =
            NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap());
        let parallel_results = executor
            .execute(
                StateStorageAdapter::new(&mut state),
                env,
                txs,
                concurrency_level,
            )
            .unwrap();
        check_execute_results(&parallel_results, name, suite);
        // Check sequential and parallel results are same.
        for (s_res, p_res) in sequential_results.iter().zip(&parallel_results) {
            assert_eq!(s_res.receipt, p_res.receipt);
        }
    }
    Ok(())
}

fn check_execute_results(results: &[TxExecutionResult], name: &str, suite: &Suite) {
    for (idx, result) in results.iter().enumerate() {
        let expect_logs = suite.logs.get(&idx.to_string()).unwrap();
        assert_eq!(
            expect_logs.len(),
            result.receipt.logs.len(),
            "name: {} tx idx {}",
            name,
            idx
        );
        // Check expected logs
        for (i, log) in result.receipt.logs.iter().enumerate() {
            let expect_log = expect_logs.get(&i.to_string()).unwrap();
            assert_eq!(
                expect_log.data, log.data.data,
                "name: {} tx idx {} log idx {}",
                name, idx, i
            );
            assert_eq!(
                expect_log.topics,
                log.topics(),
                "name: {} tx idx {} log idx {}",
                name,
                idx,
                i
            );
            assert_eq!(
                expect_log.address, log.address,
                "name: {} tx idx {} log idx {}",
                name, idx, i
            );
        }
    }
    // Check the post state account nonce.
    if let Some(post_state) = &suite.post
        && let Some(result) = results.last()
    {
        let db_state = result.state();
        for (address, expect_account) in post_state {
            let db_account = db_state.get(address).cloned().unwrap_or_default();
            if expect_account.nonce != db_account.info.nonce {
                let kind = TestErrorKind::AccountMismatch {
                    got: (*address, db_account.info.balance, db_account.info.nonce),
                    expected: (*address, expect_account.balance, expect_account.nonce),
                };
                panic!("{kind:?}");
            }
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Run(run_args) => {
            for path in &run_args.path {
                let tests = find_all_json_tests(path);
                let pb = ProgressBar::new(tests.len() as u64);
                pb.set_draw_target(ProgressDrawTarget::stdout());
                let builder = std::thread::Builder::new();
                let handle = builder
                    .spawn(move || {
                        for test_path in tests {
                            match execute_test(&test_path) {
                                Ok(_) => pb.inc(1),
                                Err(e) => panic!("Test failed: {e:?}"),
                            }
                        }
                        pb.finish_with_message("All tests completed");
                    })
                    .unwrap();
                handle.join().unwrap();
            }
            Ok(())
        }
    }
}
