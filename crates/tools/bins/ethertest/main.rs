use alloy_eips::eip4844::calc_excess_blob_gas;
use alloy_evm::EvmEnv;
use alloy_rlp::{RlpEncodable, RlpMaxEncodedLen};
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use either::Either;
use hash_db::Hasher;
use indicatif::{ProgressBar, ProgressDrawTarget};
use metis_chain::state::StateStorageAdapter;
use metis_pe::{AccountInfo, ParallelExecutor};
use metis_primitives::{
    AccessList, Address, B256, Bytecode, Bytes, GAS_PER_BLOB, Log, PlainAccount,
    SignedAuthorization, SpecId, TxEnv, TxKind, U256, as_u64_saturated, keccak256,
};
use metis_tools::{SpecName, find_all_json_tests};
use plain_hasher::PlainHasher;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use revm::{
    DatabaseCommit,
    database::{CacheState, State},
};
use serde::{Deserialize, Serialize, de};
use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::exit,
};
use thiserror::Error;
use triehash::sec_trie_root;

pub const TARGET_BLOB_GAS_PER_BLOCK_CANCUN: u64 = 3 * GAS_PER_BLOB;

#[derive(Parser)]
#[command(author, version, about)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run ether state tests with given parameters
    Run(RunArgs),
}

#[derive(Args)]
struct RunArgs {
    path: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct TestSuite(pub BTreeMap<String, Test>);

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct Test {
    #[serde(default, rename = "_info")]
    pub info: Option<serde_json::Value>,
    env: TestEnv,
    transaction: TestTransaction,
    pre: HashMap<Address, TestAccountInfo>,
    post: BTreeMap<SpecName, Vec<PostStateTest>>,
    #[serde(default)]
    pub out: Option<Bytes>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestEnv {
    pub current_coinbase: Address,
    #[serde(default)]
    pub current_difficulty: U256,
    pub current_gas_limit: U256,
    pub current_number: U256,
    pub current_timestamp: U256,
    pub current_base_fee: Option<U256>,
    pub previous_hash: Option<B256>,
    pub current_random: Option<B256>,
    pub current_beacon_root: Option<B256>,
    pub current_withdrawals_root: Option<B256>,
    pub parent_blob_gas_used: Option<U256>,
    pub parent_excess_blob_gas: Option<U256>,
    pub parent_target_blobs_per_block: Option<U256>,
    pub current_excess_blob_gas: Option<U256>,
}

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestTransaction {
    pub data: Vec<Bytes>,
    pub gas_limit: Vec<U256>,
    pub gas_price: Option<U256>,
    pub nonce: U256,
    pub secret_key: B256,
    #[serde(default)]
    pub sender: Option<Address>,
    #[serde(default, deserialize_with = "deserialize_maybe_empty")]
    pub to: Option<Address>,
    pub value: Vec<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    #[serde(default)]
    pub access_lists: Vec<Option<AccessList>>,
    pub authorization_list: Option<Vec<TestAuthorization>>,
    #[serde(default)]
    pub blob_versioned_hashes: Vec<B256>,
    pub max_fee_per_blob_gas: Option<U256>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestAuthorization {
    #[serde(flatten)]
    inner: SignedAuthorization,
}

impl From<TestAuthorization> for SignedAuthorization {
    fn from(auth: TestAuthorization) -> Self {
        auth.inner
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestAccountInfo {
    pub balance: U256,
    pub code: Bytes,
    pub nonce: U256,
    pub storage: HashMap<U256, U256>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostStateTest {
    pub expect_exception: Option<String>,
    pub indexes: TestIndexes,
    pub hash: B256,
    #[serde(default)]
    pub post_state: HashMap<Address, TestAccountInfo>,
    pub logs: B256,
    pub txbytes: Option<Bytes>,
    /// Output state.
    #[serde(default)]
    pub state: HashMap<Address, TestAccountInfo>,
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TestIndexes {
    pub data: usize,
    pub gas: usize,
    pub value: usize,
}

#[derive(Debug, Error)]
#[error("Test {name} suite {suite_name:?} index {indexes:?} failed: {kind}")]
pub struct TestError {
    pub name: String,
    pub suite_name: Option<String>,
    pub spec: Option<SpecName>,
    pub indexes: Option<TestIndexes>,
    pub kind: TestErrorKind,
}

#[derive(Debug, Error)]
pub enum TestErrorKind {
    #[error("logs root mismatch: expected {expected}, got {got}")]
    LogsRootMismatch { expected: B256, got: B256 },
    #[error("state root mismatch: expected {expected}, got {got}")]
    StateRootMismatch { expected: B256, got: B256 },
    #[error("account state mismatch: expected {expected:?}, got {got:?}")]
    AccountMismatch {
        expected: (Address, HashMap<U256, U256>, U256, u64),
        got: (Address, HashMap<U256, U256>, U256, u64),
    },
    #[error("unknown private key: {0:?}")]
    UnknownPrivateKey(B256),
    #[error(transparent)]
    SerdeDeserialize(#[from] serde_json::Error),
    #[error("unexpected exception: expected {expected_exception:?}, got {got_exception:?}")]
    UnexpectedException {
        expected_exception: Option<String>,
        got_exception: Option<String>,
    },
}

fn log_rlp_hash(logs: &[Log]) -> B256 {
    let mut out = Vec::with_capacity(alloy_rlp::list_length(logs));
    alloy_rlp::encode_list(logs, &mut out);
    B256::from_slice(keccak256(&out).as_slice())
}

#[inline]
pub fn state_merkle_trie_root<'a>(
    accounts: impl IntoIterator<Item = (Address, &'a PlainAccount)>,
) -> B256 {
    trie_root(accounts.into_iter().map(|(address, acc)| {
        (
            address,
            alloy_rlp::encode_fixed_size(&TrieAccount::new(acc)),
        )
    }))
}

#[derive(RlpEncodable, RlpMaxEncodedLen)]
struct TrieAccount {
    nonce: u64,
    balance: U256,
    root_hash: B256,
    code_hash: B256,
}

impl TrieAccount {
    fn new(acc: &PlainAccount) -> Self {
        Self {
            nonce: acc.info.nonce,
            balance: acc.info.balance,
            root_hash: sec_trie_root::<KeccakHasher, _, _, _>(
                acc.storage
                    .iter()
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(k, v)| (k.to_be_bytes::<32>(), alloy_rlp::encode_fixed_size(v))),
            ),
            code_hash: acc.info.code_hash,
        }
    }
}

#[inline]
pub fn trie_root<I, A, B>(input: I) -> B256
where
    I: IntoIterator<Item = (A, B)>,
    A: AsRef<[u8]>,
    B: AsRef<[u8]>,
{
    sec_trie_root::<KeccakHasher, _, _, _>(input)
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeccakHasher;

impl Hasher for KeccakHasher {
    type Out = B256;
    type StdHasher = PlainHasher;
    const LENGTH: usize = 32;

    #[inline]
    fn hash(x: &[u8]) -> Self::Out {
        keccak256(x)
    }
}

#[inline]
fn should_skip(path: &Path) -> bool {
    let name = path.file_name().unwrap().to_str().unwrap();
    matches!(
        name,
        // JSON big int issue cases: https://github.com/ethereum/tests/issues/971
        "ValueOverflow.json"
        | "ValueOverflowParis.json"

        // Precompiles having storage is not possible
        | "RevertPrecompiledTouch_storage.json"
        | "RevertPrecompiledTouch.json"

        // Txbyte is of type 02 and we don't parse tx bytes for this test to fail.
        | "typeTwoBerlin.json"

        // Test check if gas price overflows, we handle this correctly but does not match tests specific exception.
        | "HighGasPrice.json"
        | "CREATE_HighNonce.json"
        | "CREATE_HighNonceMinus1.json"
        | "CreateTransactionHighNonce.json"

        // Skip test where basefee/accesslist/difficulty is present but it shouldn't be supported in
        // London/Berlin/TheMerge. https://github.com/ethereum/tests/blob/5b7e1ab3ffaf026d99d20b17bb30f533a2c80c8b/GeneralStateTests/stExample/eip1559.json#L130
        // It is expected to not execute these tests.
        | "basefeeExample.json"
        | "eip1559.json"
        | "mergeTest.json"
        | "emptyBlobhashList.json"

        // Test with some storage check.
        | "RevertInCreateInInit_Paris.json"
        | "RevertInCreateInInit.json"
        | "dynamicAccountOverwriteEmpty.json"
        | "dynamicAccountOverwriteEmpty_Paris.json"
        | "RevertInCreateInInitCreate2Paris.json"
        | "create2collisionStorage.json"
        | "RevertInCreateInInitCreate2.json"
        | "create2collisionStorageParis.json"
        | "InitCollision.json"
        | "InitCollisionParis.json"

        // The expected `OufOfGas` error is not filled in this
        // test case, but the test does throw an error.
        | "warm_coinbase_call_out_of_gas.json"
        | "warm_coinbase_gas_usage.json"

        // Skip evmone statetest
        | "initcode_transaction_before_prague.json"
        | "invalid_tx_non_existing_sender.json"
        | "tx_non_existing_sender.json"
        | "block_apply_withdrawal.json"
        | "block_apply_ommers_reward.json"
        | "known_block_hash.json"
        | "eip7516_blob_base_fee.json"
        | "create_tx_collision_storage.json"
        | "create_collision_storage.json"
    )
}

fn execute_test(path: &Path) -> Result<(), Box<TestError>> {
    if should_skip(path) {
        return Ok(());
    }
    let name = path.to_string_lossy().to_string();
    let s = std::fs::read_to_string(path).unwrap();
    let suite: TestSuite = serde_json::from_str(&s).map_err(|e| TestError {
        name: name.clone(),
        suite_name: None,
        spec: None,
        indexes: None,
        kind: e.into(),
    })?;

    suite.0.into_par_iter().for_each(|(suite_name, suite)| {
        // Revm comparative test
        let mut cache_state = CacheState::new(false);
        for (address, info) in suite.pre.iter() {
            let code_hash = keccak256(info.code.clone());
            let bytecode = Bytecode::new_raw(info.code.clone());
            cache_state.contracts.insert(code_hash, bytecode.clone());
            cache_state.insert_account_with_storage(
                *address,
                AccountInfo {
                    balance: info.balance,
                    code_hash,
                    code: Some(bytecode),
                    nonce: as_u64_saturated!(info.nonce),
                },
                info.storage.iter().map(|(k, v)| (*k, *v)).collect(),
            );
        }

        // Post and execution
        suite.post.par_iter().for_each(|(spec_name, tests)| {
            // Constantinople was immediately extended by Petersburg.
            // There isn't any production Constantinople transaction
            // so we don't support it and skip right to Petersburg.
            if spec_name == &SpecName::Constantinople {
                return;
            }
            let spec_id = spec_name.to_spec_id();
            for test_case in tests {
                let mut env = setup_env(&suite, spec_id).unwrap();
                if spec_id.is_enabled_in(SpecId::MERGE) && env.block_env.prevrandao.is_none() {
                    // if spec is merge and prevrandao is not set, set it to default
                    env.block_env.prevrandao = Some(B256::default());
                }
                let mut cache = cache_state.clone();
                cache.set_state_clear_flag(spec_id.is_enabled_in(SpecId::SPURIOUS_DRAGON));
                let mut state = State::builder()
                    .with_cached_prestate(cache)
                    .with_bundle_update()
                    .build();
                let mut tx = TxEnv {
                    data: suite
                        .transaction
                        .data
                        .get(test_case.indexes.data)
                        .unwrap()
                        .clone(),
                    gas_limit: suite.transaction.gas_limit[test_case.indexes.gas].saturating_to(),
                    gas_price: suite
                        .transaction
                        .gas_price
                        .or(suite.transaction.max_fee_per_gas)
                        .unwrap_or_default()
                        .try_into()
                        .unwrap_or(u128::MAX),
                    gas_priority_fee: suite
                        .transaction
                        .max_priority_fee_per_gas
                        .map(|b| u128::try_from(b).expect("max priority fee less than u128::MAX")),
                    nonce: as_u64_saturated!(suite.transaction.nonce),
                    value: suite
                        .transaction
                        .value
                        .get(test_case.indexes.value)
                        .cloned()
                        .unwrap_or_default(),
                    blob_hashes: suite.transaction.blob_versioned_hashes.clone(),
                    max_fee_per_blob_gas: suite
                        .transaction
                        .max_fee_per_blob_gas
                        .map(|b| u128::try_from(b).expect("max fee less than u128::MAX"))
                        .unwrap_or(0),
                    kind: match suite.transaction.to {
                        Some(to) => TxKind::Call(to),
                        None => TxKind::Create,
                    },
                    access_list: suite
                        .transaction
                        .access_lists
                        .get(test_case.indexes.data)
                        .cloned()
                        .flatten()
                        .unwrap_or_default(),

                    authorization_list: suite
                        .transaction
                        .authorization_list
                        .clone()
                        .map(|auth_list| {
                            auth_list
                                .into_iter()
                                .map(|i| Either::Left(i.into()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),

                    caller: if let Some(address) = suite.transaction.sender {
                        address
                    } else {
                        let addr = recover_address(&suite.transaction.secret_key.0)
                            .ok_or_else(|| TestError {
                                name: name.to_string(),
                                suite_name: None,
                                spec: None,
                                indexes: None,
                                kind: TestErrorKind::UnknownPrivateKey(
                                    suite.transaction.secret_key,
                                ),
                            })
                            .unwrap();
                        Address::from_slice(addr.as_slice())
                    },
                    ..Default::default()
                };

                if tx.derive_tx_type().is_err() {
                    if test_case.expect_exception.is_some() {
                        return;
                    } else {
                        panic!("Invalid transaction type without expected exception");
                    }
                }
                let mut executor = ParallelExecutor::default();
                let concurrency_level =
                    NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap());

                let res = executor
                    .execute(
                        StateStorageAdapter::new(&mut state),
                        env,
                        vec![tx],
                        concurrency_level,
                    )
                    .map(|res| res[0].clone());

                // Check result and output.
                match res {
                    Ok(res) => {
                        // Check the expect exception.
                        if test_case.expect_exception.is_some() && res.receipt.success {
                            panic!(
                                "{:?}",
                                TestError {
                                    name: name.to_string(),
                                    suite_name: Some(suite_name.to_string()),
                                    spec: Some(*spec_name),
                                    indexes: Some(test_case.indexes.clone()),
                                    kind: TestErrorKind::UnexpectedException {
                                        expected_exception: test_case.expect_exception.clone(),
                                        got_exception: None,
                                    },
                                }
                            );
                        }
                        // Check the logs root.
                        let logs_root = log_rlp_hash(&res.receipt.logs);
                        if logs_root != test_case.logs {
                            let kind = TestErrorKind::LogsRootMismatch {
                                got: logs_root,
                                expected: test_case.logs,
                            };
                            panic!(
                                "{:?}",
                                TestError {
                                    name: name.to_string(),
                                    suite_name: Some(suite_name.to_string()),
                                    spec: Some(*spec_name),
                                    indexes: Some(test_case.indexes.clone()),
                                    kind,
                                }
                            );
                        }
                        state.commit(res.state);
                        // Check the account state diff.
                        if !test_case.state.is_empty() {
                            for (address, expect_account) in &test_case.state {
                                let db_account = state
                                    .load_cache_account(*address)
                                    .unwrap()
                                    .account
                                    .clone()
                                    .unwrap_or_default();
                                let got_storage: HashMap<U256, U256> = db_account
                                    .storage
                                    .iter()
                                    .map(|(k, v)| (*k, *v))
                                    .filter(|(_, v)| !v.is_zero())
                                    .collect();
                                let nonce = as_u64_saturated!(expect_account.nonce);
                                if expect_account.storage.len() != got_storage.len()
                                    || expect_account.storage != got_storage
                                    || expect_account.balance != db_account.info.balance
                                    || nonce != db_account.info.nonce
                                {
                                    let kind = TestErrorKind::AccountMismatch {
                                        got: (
                                            *address,
                                            got_storage,
                                            db_account.info.balance,
                                            db_account.info.nonce,
                                        ),
                                        expected: (
                                            *address,
                                            expect_account.storage.clone(),
                                            expect_account.balance,
                                            nonce,
                                        ),
                                    };
                                    panic!(
                                        "{:?}",
                                        TestError {
                                            name: name.to_string(),
                                            suite_name: Some(suite_name.to_string()),
                                            spec: Some(*spec_name),
                                            indexes: Some(test_case.indexes.clone()),
                                            kind,
                                        }
                                    );
                                }
                            }
                        }
                        // Read all account state from the database.
                        let state_root = state_merkle_trie_root(state.cache.trie_account());
                        if state_root != test_case.hash {
                            let kind = TestErrorKind::StateRootMismatch {
                                got: state_root,
                                expected: test_case.hash,
                            };
                            panic!(
                                "{:?}",
                                TestError {
                                    name: name.to_string(),
                                    suite_name: Some(suite_name.to_string()),
                                    spec: Some(*spec_name),
                                    indexes: Some(test_case.indexes.clone()),
                                    kind,
                                }
                            );
                        }
                    }
                    Err(error) => {
                        if test_case.expect_exception.is_none() {
                            panic!(
                                "{:?}",
                                TestError {
                                    name: name.to_string(),
                                    suite_name: Some(suite_name.to_string()),
                                    spec: Some(*spec_name),
                                    indexes: Some(test_case.indexes.clone()),
                                    kind: TestErrorKind::UnexpectedException {
                                        expected_exception: test_case.expect_exception.clone(),
                                        got_exception: Some(error.to_string()),
                                    },
                                }
                            );
                        }
                    }
                }
            }
        });
    });

    Ok(())
}

fn setup_env(test: &Test, spec_id: SpecId) -> Result<EvmEnv, Box<TestError>> {
    let mut env: EvmEnv<SpecId, revm::context::BlockEnv> = EvmEnv::default();
    env.cfg_env.chain_id = 1;
    env.cfg_env.spec = spec_id;
    // Configure max blobs per spec
    if env.cfg_env.spec.is_enabled_in(SpecId::PRAGUE) {
        env.cfg_env.set_max_blobs_per_tx(9);
    } else {
        env.cfg_env.set_max_blobs_per_tx(6);
    }
    env.block_env.number = test.env.current_number;
    env.block_env.beneficiary = test.env.current_coinbase;
    env.block_env.gas_limit = test.env.current_gas_limit.try_into().unwrap_or(u64::MAX);
    env.block_env.timestamp = test.env.current_timestamp;
    env.block_env.basefee = test
        .env
        .current_base_fee
        .unwrap_or_default()
        .try_into()
        .unwrap_or(u64::MAX);
    env.block_env.difficulty = test.env.current_difficulty;
    env.block_env.prevrandao = test.env.current_random;
    // EIP-4844
    if let Some(current_excess_blob_gas) = test.env.current_excess_blob_gas {
        env.block_env.set_blob_excess_gas_and_price(
            current_excess_blob_gas.to(),
            revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN,
        );
    } else if let (Some(parent_blob_gas_used), Some(parent_excess_blob_gas)) = (
        test.env.parent_blob_gas_used,
        test.env.parent_excess_blob_gas,
    ) {
        env.block_env.set_blob_excess_gas_and_price(
            calc_excess_blob_gas(parent_blob_gas_used.to(), parent_excess_blob_gas.to()),
            revm::primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN,
        );
    }
    Ok(env)
}

fn recover_address(private_key: &[u8]) -> Option<Address> {
    use k256::ecdsa::SigningKey;

    let key = SigningKey::from_slice(private_key).ok()?;
    let public_key = key.verifying_key().to_encoded_point(false);
    Some(Address::from_raw_public_key(&public_key.as_bytes()[1..]))
}

pub fn deserialize_maybe_empty<'de, D>(deserializer: D) -> Result<Option<Address>, D::Error>
where
    D: de::Deserializer<'de>,
{
    let string = String::deserialize(deserializer)?;
    if string.is_empty() {
        Ok(None)
    } else {
        string.parse().map_err(de::Error::custom).map(Some)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Run(run_args) => {
            let mut total_errors = vec![];
            for path in &run_args.path {
                let tests = find_all_json_tests(path);
                let pb = ProgressBar::new(tests.len() as u64);
                pb.set_draw_target(ProgressDrawTarget::stdout());
                let builder = std::thread::Builder::new();
                let handle = builder.spawn(move || {
                    let mut errors = vec![];
                    for test_path in tests {
                        match execute_test(&test_path) {
                            Ok(_) => pb.inc(1),
                            Err(e) => {
                                errors.push(e);
                            }
                        }
                    }
                    errors
                })?;
                let mut errors = handle.join().unwrap();
                total_errors.append(&mut errors);
            }
            if total_errors.is_empty() {
                Ok(())
            } else {
                exit(1);
            }
        }
    }
}
