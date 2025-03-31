use reth_provider::ProviderError;
use reth_evm::{ ConfigureEvm, execute::{BlockExecutionError,  BlockValidationError}};
use reth_chainspec::{ChainSpec, EthChainSpec, MAINNET};
use reth_revm::db::{states::bundle_state::BundleRetention, State};
use reth_primitives::{ Request, BlockWithSenders, EthereumHardfork};

use pevm::{Pevm, chain::{PevmChain, PevmEthereum}, PevmError};
use pevm::Storage;

use std::{num::NonZeroUsize, sync::Arc, thread};
use std::fmt::Display;
use std::sync::Mutex;
use alloy_primitives::{BlockNumber, U256};

use revm::primitives:: {
    db::{DatabaseRef},
    ResultAndState,
    BlockEnv, TxEnv,
};

//use reth_revm::primitives::{BlockEnv, TxEnv};
use alloy_rpc_types_eth::{Receipt, Block, BlockTransactions, Header};
use reth_ethereum_forks::EthereumHardforks;
use reth_revm::DatabaseCommit;
use reth_revm::state_change::post_block_balance_increments;
use crate::hard_forks::{DAO_HARDFORK_BENEFICIARY, DAO_HARDKFORK_ACCOUNTS};

/// Helper type for the output of executing a block.
#[derive(Debug, Clone)]
struct EthExecuteOutput {
    receipts: Vec<Receipt>,
    requests: Vec<Request>,
    gas_used: u64,
}

/// Helper container type for EVM with chain spec.
#[derive(Debug, Clone)]
struct EthEvmExecutor<EvmConfig> {
    /// The chainspec
    chain_spec: Arc<ChainSpec>,
    /// How to create an EVM.
    evm_config: EvmConfig,
}

/// A basic Ethereum block executor.
///
/// Expected usage:
/// - Create a new instance of the executor.
/// - Execute the block.
#[derive(Debug)]
pub struct MetisBlockExecutor<EvmConfig, DB> {
    /// Chain specific evm config that's used to execute a block.
    executor: EthEvmExecutor<EvmConfig>,
    /// The state to use for execution
    state: DB,
    /// Parallel executor
    pevm: Pevm,
    chain: PevmEthereum,
    concurrency_level: NonZeroUsize,
}

impl<EvmConfig, DB> MetisBlockExecutor<EvmConfig, DB> {
    /// Creates a new Ethereum block executor.
    pub const fn new(
        chain_spec: Arc<ChainSpec>,
        evm_config: EvmConfig,
        state: State<DB>,
        pevm: Pevm,
        chain: PevmEthereum,
        concurrency_level: NonZeroUsize,
    ) -> Self {
        Self {
            executor: EthEvmExecutor { chain_spec, evm_config },
            state,
            pevm,
            chain,
            concurrency_level,
        }
    }

    #[inline]
    fn chain_spec(&self) -> &ChainSpec {
        &self.executor.chain_spec
    }

    /// Returns mutable reference to the state that wraps the underlying database.
    #[allow(unused)]
    fn state_mut(&mut self) -> &mut State<DB> {
        &mut self.state
    }
}

impl<EvmConfig, DB> MetisBlockExecutor<EvmConfig, DB>
where
    EvmConfig: ConfigureEvm<Header = Header>,
    DB: DatabaseRef<Error: Into<ProviderError> + Display> + Send + Sync,
{

    /// Execute a single block and apply the state changes to the internal state.
    ///
    /// Returns the receipts of the transactions in the block, the total gas used and the list of
    /// EIP-7685 [requests](Request).
    ///
    /// Returns an error if execution fails.
    fn execute_without_verification(
        &mut self,
        block: &Block,
        total_difficulty: U256,
    ) -> Result<EthExecuteOutput, BlockExecutionError> {
        // 1. prepare state on new block
        self.on_new_block(block.header.number);

        // 2. configure the evm and execute
        let spec_id = crate::config::revm_spec(
            self.chain_spec(),
            &reth_chainspec::Head {
                number: block.header.number,
                timestamp: block.header.timestamp,
                difficulty: block.header.difficulty,
                total_difficulty,
                hash: Default::default(),
            },
        );
        let txs = block.transactions.clone();
        let mut block_env = BlockEnv::default();
        let tx_envs = match &txs {
            BlockTransactions::Full(txs) => txs
                .iter()
                .map(|tx| self.chain.get_tx_env(tx))
                .collect::<Result<Vec<TxEnv>, _>>()
                .map_err(PevmError::InvalidTransaction)?,
            _ => return Err(PevmError::MissingTransactionData),
        };
        let results =
            if tx_envs.len() < self.concurrency_level.into() || block.header.gas_used < 4_000_000 {
                pevm::execute_revm_sequential(
                    &self.state,
                    &self.chain,
                    spec_id,
                    block_env,
                    tx_envs,
                    self.pevm.worker.clone())
            } else {
                self.pevm.execute_revm_parallel(
                    &self.state,
                    &self.chain,
                    spec_id,
                    block_env,
                    tx_envs,
                    self.concurrency_level,
                )
            }
                .unwrap_or_else(|err| panic!("{:?}", err));
        let mut cumulative_gas_used = 0;
        let mut receipts = Vec::with_capacity(txs.len());
        for (tx, ResultAndState { result, state }) in txs.zip(results) {
            cumulative_gas_used += result.gas_used();
            receipts.push(Receipt {
                status: result.is_success().into(),
                cumulative_gas_used,
                logs: result.into_logs(),
            });
            self.state.commit(state);
        }

        // 3. apply post execution changes
        self.post_execution(block, total_difficulty)?;

        Ok(EthExecuteOutput { receipts, requests: Vec::new(), gas_used: cumulative_gas_used })
    }

    /// Apply settings before a new block is executed.
    pub(crate) fn on_new_block(&mut self, block_number: u64) {
        // Set state clear flag if the block is after the Spurious Dragon hardfork.
        let state_clear_flag = self.chain_spec().is_spurious_dragon_active_at_block(block_number);
        self.state.set_state_clear_flag(state_clear_flag);
    }

    /// Apply post execution state changes that do not require an [EVM](Evm), such as: block
    /// rewards, withdrawals, and irregular DAO hardfork state change
    pub fn post_execution(
        &mut self,
        block: &Block,
        total_difficulty: U256,
    ) -> Result<(), BlockExecutionError> {
        let mut balance_increments =
            post_block_balance_increments(self.chain_spec(), block, total_difficulty);

        // Irregular state change at Ethereum DAO hardfork
        if self.chain_spec().fork(EthereumHardfork::Dao).transitions_at_block(block.number) {
            // drain balances from hardcoded addresses.
            let drained_balance: u128 = self
                .state
                .drain_balances(DAO_HARDKFORK_ACCOUNTS)
                .map_err(|_| BlockValidationError::IncrementBalanceFailed)?
                .into_iter()
                .sum();

            // return balance to DAO beneficiary.
            *balance_increments.entry(DAO_HARDFORK_BENEFICIARY).or_default() += drained_balance;
        }
        // increment balances
        self.state
            .increment_balances(balance_increments)
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;

        Ok(())
    }
}
