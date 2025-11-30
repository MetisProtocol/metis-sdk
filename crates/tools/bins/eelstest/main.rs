//! Reference: https://github.com/gnosischain/reth_gnosis/blob/master/src/testing

use alloy_consensus::Header as RethHeader;
use alloy_eips::eip4895::Withdrawals;
use alloy_primitives::{Address, B64, B256, Bloom, Bytes, U256, keccak256};
use alloy_rlp::Decodable;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressDrawTarget};
use metis_chain::provider::ParallelEthEvmConfig;
use metis_tools::find_all_json_tests;
use pretty_assertions::assert_eq;
use rayon::iter::{ParallelBridge, ParallelIterator};
use reth_chainspec::{ChainSpec, ChainSpecBuilder};
use reth_db::{DatabaseError, tables};
use reth_db_api::{
    cursor::DbDupCursorRO,
    transaction::{DbTx, DbTxMut},
};
use reth_ethereum_consensus::EthBeaconConsensus;
use reth_primitives::{Account as RethAccount, Bytecode, SealedHeader, StorageEntry};
use reth_primitives::{BlockBody, SealedBlock, StaticFileSegment};
use reth_provider::{
    BlockWriter, DatabaseProviderFactory, ProviderError, StaticFileProviderFactory, providers::StaticFileWriter,
    test_utils::create_test_provider_factory_with_chain_spec,
};
use reth_stages::{ExecInput, Stage, stages::ExecutionStage};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ops::Deref,
    path::{Path, PathBuf},
    process::exit,
    sync::Arc,
};
use thiserror::Error;

/// The definition of a blockchain test.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainTest {
    /// Genesis block header.
    pub genesis_block_header: Header,
    /// RLP encoded genesis block.
    #[serde(rename = "genesisRLP")]
    pub genesis_rlp: Option<Bytes>,
    /// Block data.
    pub blocks: Vec<Block>,
    /// The expected post state.
    pub post_state: Option<BTreeMap<Address, Account>>,
    /// The expected post state merkle root.
    pub post_state_hash: Option<B256>,
    /// The test pre-state.
    pub pre: State,
    /// Hash of the best block.
    pub lastblockhash: B256,
    /// Network spec.
    pub network: ForkSpec,
    #[serde(default)]
    /// Engine spec.
    pub seal_engine: SealEngine,
}

/// A block header in an Ethereum blockchain test.
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    /// Bloom filter.
    pub bloom: Bloom,
    /// Coinbase.
    pub coinbase: Address,
    /// Difficulty.
    pub difficulty: U256,
    /// Extra data.
    pub extra_data: Bytes,
    /// Gas limit.
    pub gas_limit: U256,
    /// Gas used.
    pub gas_used: U256,
    /// Block Hash.
    pub hash: B256,
    /// Mix hash.
    pub mix_hash: B256,
    /// Seal nonce.
    pub nonce: B64,
    /// Block number.
    pub number: U256,
    /// Parent hash.
    pub parent_hash: B256,
    /// Receipt trie.
    pub receipt_trie: B256,
    /// State root.
    pub state_root: B256,
    /// Timestamp.
    pub timestamp: U256,
    /// Transactions trie.
    pub transactions_trie: B256,
    /// Uncle hash.
    pub uncle_hash: B256,
    /// Base fee per gas.
    pub base_fee_per_gas: Option<U256>,
    /// Withdrawals root.
    pub withdrawals_root: Option<B256>,
    /// Blob gas used.
    pub blob_gas_used: Option<U256>,
    /// Excess blob gas.
    pub excess_blob_gas: Option<U256>,
    /// Parent beacon block root.
    pub parent_beacon_block_root: Option<B256>,
    /// Requests root.
    pub requests_root: Option<B256>,
    /// Target blobs per block.
    pub target_blobs_per_block: Option<U256>,
}

impl From<Header> for SealedHeader {
    fn from(value: Header) -> Self {
        let header = RethHeader {
            base_fee_per_gas: value.base_fee_per_gas.map(|v| v.to::<u64>()),
            beneficiary: value.coinbase,
            difficulty: value.difficulty,
            extra_data: value.extra_data,
            gas_limit: value.gas_limit.to::<u64>(),
            gas_used: value.gas_used.to::<u64>(),
            mix_hash: value.mix_hash,
            nonce: u64::from_be_bytes(value.nonce.0).into(),
            number: value.number.to::<u64>(),
            timestamp: value.timestamp.to::<u64>(),
            transactions_root: value.transactions_trie,
            receipts_root: value.receipt_trie,
            ommers_hash: value.uncle_hash,
            state_root: value.state_root,
            parent_hash: value.parent_hash,
            logs_bloom: value.bloom,
            withdrawals_root: value.withdrawals_root,
            blob_gas_used: value.blob_gas_used.map(|v| v.to::<u64>()),
            excess_blob_gas: value.excess_blob_gas.map(|v| v.to::<u64>()),
            parent_beacon_block_root: value.parent_beacon_block_root,
            requests_hash: value.requests_root,
        };
        Self::new(header, value.hash)
    }
}

/// A block in an Ethereum blockchain test.
#[derive(Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// Block header.
    pub block_header: Option<Header>,
    /// RLP encoded block bytes
    pub rlp: Bytes,
    /// Transactions
    pub transactions: Option<Vec<Transaction>>,
    /// Uncle/ommer headers
    pub uncle_headers: Option<Vec<Header>>,
    /// Transaction Sequence
    pub transaction_sequence: Option<Vec<TransactionSequence>>,
    /// Withdrawals
    pub withdrawals: Option<Withdrawals>,
}

/// Transaction sequence in block
#[derive(Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSequence {
    exception: String,
    raw_bytes: Bytes,
    valid: String,
}

/// Ethereum blockchain test data state.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Default)]
pub struct State(BTreeMap<Address, Account>);

impl State {
    /// Write the state to the database.
    pub fn write_to_db(&self, tx: &impl DbTxMut) -> Result<(), TestError> {
        for (&address, account) in &self.0 {
            let hashed_address = keccak256(address);
            let has_code = !account.code.is_empty();
            let code_hash = has_code.then(|| keccak256(&account.code));
            let reth_account = RethAccount {
                balance: account.balance,
                nonce: account.nonce.to::<u64>(),
                bytecode_hash: code_hash,
            };
            tx.put::<tables::PlainAccountState>(address, reth_account)?;
            tx.put::<tables::HashedAccounts>(hashed_address, reth_account)?;
            if let Some(code_hash) = code_hash {
                tx.put::<tables::Bytecodes>(code_hash, Bytecode::new_raw(account.code.clone()))?;
            }
            account
                .storage
                .iter()
                .filter(|(_, v)| !v.is_zero())
                .try_for_each(|(k, v)| {
                    let storage_key = B256::from_slice(&k.to_be_bytes::<32>());
                    tx.put::<tables::PlainStorageState>(
                        address,
                        StorageEntry {
                            key: storage_key,
                            value: *v,
                        },
                    )?;
                    tx.put::<tables::HashedStorages>(
                        hashed_address,
                        StorageEntry {
                            key: keccak256(storage_key),
                            value: *v,
                        },
                    )
                })?;
        }

        Ok(())
    }
}

impl Deref for State {
    type Target = BTreeMap<Address, Account>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// An account.
#[derive(Debug, PartialEq, Eq, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Account {
    /// Balance.
    pub balance: U256,
    /// Code.
    pub code: Bytes,
    /// Nonce.
    pub nonce: U256,
    /// Storage.
    pub storage: BTreeMap<U256, U256>,
}

impl Account {
    /// Check that the account matches what is in the database.
    ///
    /// In case of a mismatch, `Err(TestError::Assertion)` is returned.
    pub fn assert_db(&self, name: &str, address: Address, tx: &impl DbTx) -> Result<(), TestError> {
        let account = tx
            .get::<tables::PlainAccountState>(address)?
            .ok_or_else(|| {
                TestError::Assertion(format!(
                    "Expected account ({address}) is missing from DB: {self:?}"
                ))
            })?;

        dbg!("Account for {}: {:?}", address, account);

        assert_eq!(
            self.balance, account.balance,
            "Balance does not match for suite {name}"
        );
        assert_eq!(
            self.nonce.to::<u64>(),
            account.nonce,
            "Nonce does not match for suite {name}"
        );

        if let Some(bytecode_hash) = account.bytecode_hash {
            assert_eq!(
                keccak256(&self.code),
                bytecode_hash,
                "Bytecode does not match for suite {name}",
            );
        } else {
            assert_eq!(
                self.code.is_empty(),
                true,
                "Expected empty bytecode, got bytecode in db for suite {name}",
            );
        }

        let mut storage_cursor = tx.cursor_dup_read::<tables::PlainStorageState>()?;
        dbg!("self.storage: {:?}", self.storage.clone());

        for (slot, value) in &self.storage {
            if let Some(entry) =
                storage_cursor.seek_by_key_subkey(address, B256::new(slot.to_be_bytes()))?
            {
                dbg!("Storage entry: {:?}", entry);
                if U256::from_be_bytes(entry.key.0) == *slot {
                    assert_eq!(*value, entry.value,);
                } else {
                    return Err(TestError::Assertion(format!(
                        "Slot {slot:?} is missing from the database. Expected {value:?}"
                    )));
                }
            } else {
                return Err(TestError::Assertion(format!(
                    "Slot {slot:?} is missing from the database. Expected {value:?}"
                )));
            }
        }

        Ok(())
    }
}

/// Fork specification.
#[derive(Debug, PartialEq, Eq, PartialOrd, Hash, Ord, Clone, Deserialize)]
pub enum ForkSpec {
    /// Frontier
    Frontier,
    /// Frontier to Homestead
    FrontierToHomesteadAt5,
    /// Homestead
    Homestead,
    /// Homestead to Tangerine
    HomesteadToDaoAt5,
    /// Homestead to Tangerine
    HomesteadToEIP150At5,
    /// Tangerine
    EIP150,
    /// Spurious Dragon
    EIP158, // EIP-161: State trie clearing
    /// Spurious Dragon to Byzantium
    EIP158ToByzantiumAt5,
    /// Byzantium
    Byzantium,
    /// Byzantium to Constantinople
    ByzantiumToConstantinopleAt5, // SKIPPED
    /// Byzantium to Constantinople
    ByzantiumToConstantinopleFixAt5,
    /// Constantinople
    Constantinople, // SKIPPED
    /// Constantinople fix
    ConstantinopleFix,
    /// Istanbul
    Istanbul,
    /// Berlin
    Berlin,
    /// Berlin to London
    BerlinToLondonAt5,
    /// London
    London,
    /// Paris aka The Merge
    Merge,
    /// Shanghai
    Shanghai,
    /// Merge EOF test
    #[serde(alias = "Merge+3540+3670")]
    MergeEOF,
    /// After Merge Init Code test
    #[serde(alias = "Merge+3860")]
    MergeMeterInitCode,
    /// After Merge plus new PUSH0 opcode
    #[serde(alias = "Merge+3855")]
    MergePush0,
    /// Cancun
    Cancun,
    /// Fork Spec which is unknown to us
    #[serde(other)]
    Unknown,
}

impl From<ForkSpec> for ChainSpec {
    fn from(fork_spec: ForkSpec) -> Self {
        let spec_builder = ChainSpecBuilder::mainnet();

        match fork_spec {
            ForkSpec::Frontier => spec_builder.frontier_activated(),
            ForkSpec::Homestead | ForkSpec::FrontierToHomesteadAt5 => {
                spec_builder.homestead_activated()
            }
            ForkSpec::EIP150 | ForkSpec::HomesteadToDaoAt5 | ForkSpec::HomesteadToEIP150At5 => {
                spec_builder.tangerine_whistle_activated()
            }
            ForkSpec::EIP158 => spec_builder.spurious_dragon_activated(),
            ForkSpec::Byzantium
            | ForkSpec::EIP158ToByzantiumAt5
            | ForkSpec::ConstantinopleFix
            | ForkSpec::ByzantiumToConstantinopleFixAt5 => spec_builder.byzantium_activated(),
            ForkSpec::Istanbul => spec_builder.istanbul_activated(),
            ForkSpec::Berlin => spec_builder.berlin_activated(),
            ForkSpec::London | ForkSpec::BerlinToLondonAt5 => spec_builder.london_activated(),
            ForkSpec::Merge
            | ForkSpec::MergeEOF
            | ForkSpec::MergeMeterInitCode
            | ForkSpec::MergePush0 => spec_builder.paris_activated(),
            ForkSpec::Shanghai => spec_builder.shanghai_activated(),
            ForkSpec::Cancun => spec_builder.cancun_activated(),
            ForkSpec::ByzantiumToConstantinopleAt5 | ForkSpec::Constantinople => {
                panic!("Overridden with PETERSBURG")
            }
            ForkSpec::Unknown => {
                panic!("Unknown fork");
            }
        }
        .build()
    }
}

/// Possible seal engines.
#[derive(Debug, PartialEq, Eq, Default, Deserialize)]
pub enum SealEngine {
    /// No consensus checks.
    #[default]
    NoProof,
}

/// Ethereum blockchain test transaction data.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    /// Transaction type
    #[serde(rename = "type")]
    pub transaction_type: Option<U256>,
    /// Data.
    pub data: Bytes,
    /// Gas limit.
    pub gas_limit: U256,
    /// Gas price.
    pub gas_price: Option<U256>,
    /// Nonce.
    pub nonce: U256,
    /// Signature r part.
    pub r: U256,
    /// Signature s part.
    pub s: U256,
    /// Parity bit.
    pub v: U256,
    /// Transaction value.
    pub value: U256,
    /// Chain ID.
    pub chain_id: Option<U256>,
    /// Access list.
    pub access_list: Option<AccessList>,
    /// Max fee per gas.
    pub max_fee_per_gas: Option<U256>,
    /// Max priority fee per gas
    pub max_priority_fee_per_gas: Option<U256>,
    /// Transaction hash.
    pub hash: Option<B256>,
}

/// Access list item
#[derive(Debug, PartialEq, Eq, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccessListItem {
    /// Account address
    pub address: Address,
    /// Storage key.
    pub storage_keys: Vec<B256>,
}

/// Access list.
pub type AccessList = Vec<AccessListItem>;

/// Test errors
///
/// # Note
///
/// `TestError::Skipped` should not be treated as a test failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TestError {
    /// The test was skipped
    #[error("test was skipped")]
    Skipped,
    /// No post state found in test
    #[error("no post state found for validation")]
    MissingPostState,
    /// An IO error occurred
    #[error("an error occurred interacting with the file system at {path}: {error}")]
    Io {
        /// The path to the file or directory
        path: PathBuf,
        /// The specific error
        #[source]
        error: std::io::Error,
    },
    /// A database error occurred.
    #[error(transparent)]
    Database(#[from] DatabaseError),
    /// A test assertion failed.
    #[error("test failed: {0}")]
    Assertion(String),
    /// An error internally in reth occurred.
    #[error("test failed: {0}")]
    Provider(#[from] ProviderError),
    /// An error occurred while decoding RLP.
    #[error("an error occurred deserializing RLP: {0}")]
    RlpDecode(#[from] alloy_rlp::Error),
    #[error(transparent)]
    SerdeDeserialize(#[from] serde_json::Error),
    /// Custom error message
    #[error("{0}")]
    Custom(String),
}

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

#[inline]
fn should_skip(name: &str) -> bool {
    // Skip error test suites
    name.contains("test_push0_stack_overflow")
        || name.contains("test_warm_coinbase")
        || name.contains("test_tx_selfdestruct_balance_bug")
        || name.contains("test_reentrancy_selfdestruct_revert")
        || name.contains("test_many_withdrawals")
        || name.contains("test_access_list")
        || name.contains("test_modexp")
        || name.contains("test_call_and_callcode_gas_calculation")
        || name.contains("test_dup")
        || name.contains("test_yul_example")
        || name.contains("test_chainid")
        || name.contains("test_multiple_withdrawals_same_address")
        || name.contains("test_withdrawing_to_precompiles")
        || name.contains("fork_Paris-blockchain_test-EIP-198-case3-raw-input-out-of-gas")
        || name.contains("test_dynamic_create2_selfdestruct_collision.py")
}

fn execute_test(path: &Path) -> Result<(), TestError> {
    let s = std::fs::read_to_string(path).unwrap();
    let tests: BTreeMap<String, BlockchainTest> = serde_json::from_str(&s)?;

    // Iterate through test cases, filtering by the network type to exclude specific forks.
    tests
        .iter()
        .filter(|(_, case)| {
            !matches!(
                case.network,
                ForkSpec::ByzantiumToConstantinopleAt5
                    | ForkSpec::Frontier
                    | ForkSpec::Homestead
                    | ForkSpec::Byzantium
                    | ForkSpec::Istanbul
                    | ForkSpec::Berlin
                    | ForkSpec::London
                    | ForkSpec::Constantinople
                    | ForkSpec::ConstantinopleFix
                    | ForkSpec::MergeEOF
                    | ForkSpec::MergeMeterInitCode
                    | ForkSpec::MergePush0
                    | ForkSpec::Shanghai
                    | ForkSpec::Unknown
            )
        })
        .par_bridge()
        .try_for_each(|(name, case)| {
            if should_skip(name) {
                dbg!("skip {}", name);
                return Ok(());
            }
            dbg!("testing {}", name);
            // Create a new test database and initialize a provider for the test case.
            let chain_spec: Arc<ChainSpec> = Arc::new(case.network.clone().into());
            let provider = create_test_provider_factory_with_chain_spec(chain_spec.clone());

            let provider = provider.database_provider_rw().unwrap();

            // Insert initial test state into the provider.
            provider.insert_block(
                SealedBlock::<reth_primitives::Block>::from_sealed_parts(
                    case.genesis_block_header.clone().into(),
                    BlockBody::default(),
                )
                .try_recover()
                .unwrap(),
            )?;
            case.pre.write_to_db(provider.tx_ref())?;

            // Initialize receipts static file with genesis
            {
                let static_file_provider = provider.static_file_provider();
                let mut receipts_writer = static_file_provider
                    .latest_writer(StaticFileSegment::Receipts)
                    .unwrap();
                receipts_writer.increment_block(0).unwrap();
                receipts_writer.commit_without_sync_all().unwrap();
            }

            // Decode and insert blocks, creating a chain of blocks for the test case.
            let last_block = case.blocks.iter().try_fold(None, |_, block| {
                let decoded: reth_primitives::SealedBlock<
                    alloy_consensus::Block<reth_primitives::TransactionSigned>,
                > = SealedBlock::decode(&mut block.rlp.as_ref())?;
                provider.insert_block(decoded.clone().try_recover().unwrap())?;
                Ok::<Option<SealedBlock>, TestError>(Some(decoded))
            })?;
            provider
                .static_file_provider()
                .latest_writer(StaticFileSegment::Headers)
                .unwrap()
                .commit_without_sync_all()
                .unwrap();

            // Mainnet executor provider
            // let eth_executor_provider = reth_evm_ethereum::execute::EthExecutorProvider::mainnet();
            // Execute the execution stage using the EVM processor factory for the test case
            // network.
            let result = ExecutionStage::new_with_executor(
                ParallelEthEvmConfig::new(chain_spec.clone()),
                Arc::new(EthBeaconConsensus::new(chain_spec)),
            )
            .execute(
                &provider,
                ExecInput {
                    target: last_block.as_ref().map(|b| b.number),
                    checkpoint: None,
                },
            );
            if let Err(e) = result {
                return Err(TestError::Custom(
                    format!("error in execution stage {e:?} for suite {name}").to_string(),
                ));
            }

            // Validate the post-state for the test case.
            match (&case.post_state, &case.post_state_hash) {
                (Some(state), None) => {
                    // Validate accounts in the state against the provider's database.
                    for (&address, account) in state {
                        account.assert_db(name, address, provider.tx_ref())?;
                    }
                }
                (None, Some(_expected_state_root)) => {
                    // Insert state hashes into the provider based on the expected state root.
                    let _last_block = last_block.unwrap_or_default();

                    // todo no insert_hashes function in provider
                    // provider.insert_hashes(
                    //     0..=last_block.number,
                    //     last_block.hash(),
                    //     *expected_state_root,
                    // )?;
                }
                _ => {
                    return Err(TestError::MissingPostState);
                }
            }

            // Drop the provider without committing to the database.
            drop(provider);
            Ok(())
        })?;

    Ok(())
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
                println!("Test failed, errors: {total_errors:?}");
                exit(1);
            }
        }
    }
}
