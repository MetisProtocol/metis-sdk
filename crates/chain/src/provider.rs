use crate::state::StateStorageAdapter;
use alloy_consensus::Header;
use alloy_eips::eip7685::Requests;
use alloy_evm::block::{
    BlockExecutionError, BlockValidationError, CommitChanges, SystemCaller,
    state_changes::post_block_balance_increments,
};
use alloy_evm::eth::dao_fork::DAO_HARDFORK_BENEFICIARY;
use alloy_evm::eth::{dao_fork, eip6110};
use alloy_evm::{Database, FromRecoveredTx, FromTxWithEncoded};
use alloy_hardforks::EthereumHardfork;
use alloy_rpc_types_engine::ExecutionData;
use metis_primitives::{CfgEnv, ExecutionResult, SpecId, TxEnv};
use reth::api::{FullNodeTypes, NodeTypes};
use reth::builder::BuilderContext;
use reth::builder::components::ExecutorBuilder;
use reth::{providers::BlockExecutionResult, revm::db::State};
use reth_chainspec::{ChainSpec, EthChainSpec, Hardforks};
use reth_ethereum_primitives::{Block, EthPrimitives, Receipt, TransactionSigned};
use reth_evm::TransactionEnv;
use reth_evm::block::{ExecutableTx, InternalBlockExecutionError};
use reth_evm::eth::spec::EthExecutorSpec;
use reth_evm::eth::{EthBlockExecutionCtx, EthBlockExecutor, EthBlockExecutorFactory};
use reth_evm::precompiles::PrecompilesMap;
use reth_evm::{
    ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
};
use reth_evm::{EthEvmFactory, Evm, EvmEnv, EvmFactory, NextBlockEnvAttributes};
use reth_evm::{OnStateHook, execute::BlockExecutor};
pub use reth_evm_ethereum::EthEvmConfig;
use reth_evm_ethereum::{EthBlockAssembler, RethReceiptBuilder};
use reth_primitives_traits::{SealedBlock, SealedHeader};
use revm::{DatabaseCommit, context::result::ResultAndState};
use std::convert::Infallible;
use std::fmt::Debug;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Ethereum-related EVM configuration with the parallel executor.
#[derive(Debug, Clone)]
pub struct ParallelEthEvmConfig<C = ChainSpec, EvmFactory = EthEvmFactory> {
    pub config: EthEvmConfig<C, EvmFactory>,
}

impl<ChainSpec> ParallelEthEvmConfig<ChainSpec> {
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            config: EthEvmConfig::new(chain_spec),
        }
    }
}

impl<ChainSpec, EvmF> ConfigureEvm for ParallelEthEvmConfig<ChainSpec, EvmF>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
    EvmF: EvmFactory<
            Tx: TransactionEnv
                    + FromRecoveredTx<TransactionSigned>
                    + FromTxWithEncoded<TransactionSigned>,
            Spec = SpecId,
            Precompiles = PrecompilesMap,
        > + Clone
        + Debug
        + Send
        + Sync
        + Unpin
        + 'static,
{
    type Primitives = EthPrimitives;
    type Error = Infallible;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = EthBlockExecutorFactory<RethReceiptBuilder, Arc<ChainSpec>, EvmF>;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.config.executor_factory
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        &self.config.block_assembler
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnv, Self::Error> {
        self.config.evm_env(header)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnv, Self::Error> {
        self.config.next_evm_env(parent, attributes)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<EthBlockExecutionCtx<'a>, Self::Error> {
        self.config.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<EthBlockExecutionCtx<'_>, Self::Error> {
        self.config.context_for_next_block(parent, attributes)
    }
}

impl ConfigureEngineEvm<ExecutionData> for ParallelEthEvmConfig<ChainSpec>
where
    ChainSpec: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
{
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> EvmEnvFor<Self> {
        self.config.evm_env_for_payload(payload)
    }

    fn context_for_payload<'a>(&self, payload: &'a ExecutionData) -> ExecutionCtxFor<'a, Self> {
        self.config.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(&self, payload: &ExecutionData) -> impl ExecutableTxIterator<Self> {
        self.config.tx_iterator_for_payload(payload)
    }
}

/// Parallel block executor for Ethereum.
pub struct ParallelBlockExecutor<'a, Evm, Spec> {
    /// Reference to the specification object.
    spec: Spec,
    /// Eth original executor.
    executor: EthBlockExecutor<'a, Evm, Spec, RethReceiptBuilder>,
}

impl<'db, DB, E, Spec> BlockExecutor for ParallelBlockExecutor<'_, E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv>,
    Spec: EthExecutorSpec + Clone,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.executor.apply_pre_execution_changes()
    }

    fn execute_transaction_with_result_closure(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as reth_evm::Evm>::HaltReason>),
    ) -> Result<u64, BlockExecutionError> {
        self.executor.execute_transaction_with_result_closure(tx, f)
    }

    fn execute_transaction_with_commit_condition(
        &mut self,
        tx: impl ExecutableTx<Self>,
        f: impl FnOnce(&ExecutionResult<<Self::Evm as Evm>::HaltReason>) -> CommitChanges,
    ) -> Result<Option<u64>, BlockExecutionError> {
        self.executor
            .execute_transaction_with_commit_condition(tx, f)
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        self.executor.finish()
    }

    fn set_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.executor.set_state_hook(hook);
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.executor.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.executor.evm()
    }

    /// Executes all transactions in a block, applying pre and post execution changes.
    fn execute_block(
        mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Self::Receipt>, BlockExecutionError>
    where
        Self: Sized,
    {
        self.apply_pre_execution_changes()?;
        self.execute_transactions(transactions)
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<ResultAndState<<Self::Evm as Evm>::HaltReason>, BlockExecutionError> {
        self.executor.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(
        &mut self,
        output: ResultAndState<<Self::Evm as Evm>::HaltReason>,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        self.executor.commit_transaction(output, tx)
    }
}

impl<'db, DB, E, Spec> ParallelBlockExecutor<'_, E, Spec>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv>,
    Spec: EthExecutorSpec + Clone,
{
    pub fn execute_transactions(
        &mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {
        // Execute block transactions parallel
        let parallel_execute_result = self.execute(transactions)?;

        // Calculate requests
        let receipts = parallel_execute_result.receipts;
        let requests = self.calc_requests(receipts.clone())?;

        // Governance reward for full block, ommers...
        self.post_execution()?;

        // Assemble new block execution result, there is no dump receipt
        let results = BlockExecutionResult {
            receipts,
            requests,
            gas_used: parallel_execute_result.gas_used,
        };

        Ok(results)
    }

    pub fn execute(
        &mut self,
        transactions: impl IntoIterator<Item = impl ExecutableTx<Self>>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {
        let state_clear_flag = self.spec.is_spurious_dragon_active_at_block(
            self.evm().block().number.try_into().unwrap_or(u64::MAX),
        );
        let evm_env = EvmEnv::new(CfgEnv::default(), self.evm().block().clone());
        let db = self.evm_mut().db_mut();
        db.set_state_clear_flag(state_clear_flag);
        let mut parallel_executor = metis_pe::ParallelExecutor::default();
        let results = parallel_executor.execute(
            StateStorageAdapter::new(db),
            evm_env,
            transactions
                .into_iter()
                .map(|tx| tx.to_tx_env())
                .collect::<Vec<TxEnv>>(),
            NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );

        let mut total_gas_used: u64 = 0;
        let receipts = results
            .map_err(|err| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(Box::new(err)))
            })?
            .into_iter()
            .map(|r| {
                self.evm_mut().db_mut().commit(r.state);
                total_gas_used += &r.receipt.cumulative_gas_used;
                r.receipt
            })
            .collect();

        Ok(BlockExecutionResult {
            receipts,
            gas_used: total_gas_used,
            requests: Requests::default(),
        })
    }

    fn post_execution(&mut self) -> Result<(), BlockExecutionError> {
        let mut balance_increments = post_block_balance_increments(
            &self.spec,
            self.evm().block(),
            self.executor.ctx.ommers,
            self.executor.ctx.withdrawals.as_deref(),
        );
        // Irregular state change at Ethereum DAO hardfork
        if self
            .spec
            .ethereum_fork_activation(EthereumHardfork::Dao)
            .transitions_at_block(self.evm().block().number.try_into().unwrap_or(u64::MAX))
        {
            // drain balances from hardcoded addresses.
            let drained_balance: u128 = self
                .evm_mut()
                .db_mut()
                .drain_balances(dao_fork::DAO_HARDFORK_ACCOUNTS)
                .map_err(|_| BlockValidationError::IncrementBalanceFailed)?
                .into_iter()
                .sum();

            // return balance to DAO beneficiary.
            *balance_increments
                .entry(DAO_HARDFORK_BENEFICIARY)
                .or_default() += drained_balance;
        }
        // increment balances
        self.evm_mut()
            .db_mut()
            .increment_balances(balance_increments)
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;

        Ok(())
    }

    fn calc_requests(&mut self, receipts: Vec<Receipt>) -> Result<Requests, BlockExecutionError> {
        let evm = self.executor.evm_mut();
        let block = evm.block();
        let requests = if self
            .spec
            .is_prague_active_at_timestamp(block.timestamp.try_into().unwrap_or(u64::MAX))
        {
            // Collect all EIP-6110 deposits
            let deposit_requests =
                eip6110::parse_deposits_from_receipts(self.spec.clone(), &receipts)?;

            let mut requests = Requests::default();

            if !deposit_requests.is_empty() {
                requests.push_request_with_type(eip6110::DEPOSIT_REQUEST_TYPE, deposit_requests);
            }
            let mut system_caller = SystemCaller::new(self.spec.clone());
            requests.extend(system_caller.apply_post_execution_changes(evm)?);
            requests
        } else {
            Requests::default()
        };

        Ok(requests)
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = ParallelEthEvmConfig;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(ParallelEthEvmConfig::new(ctx.chain_spec()))
    }
}
