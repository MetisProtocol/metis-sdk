use crate::state::StateStorageAdapter;
use alloy_eips::eip7685::Requests;
use alloy_evm::block::state_changes::post_block_balance_increments;
use alloy_evm::block::{BlockExecutionError, BlockValidationError, SystemCaller};
use alloy_evm::eth::dao_fork::DAO_HARDFORK_BENEFICIARY;
use alloy_evm::eth::{dao_fork, eip6110};
use alloy_evm::{Database, IntoTxEnv};
use alloy_hardforks::EthereumHardfork;
use metis_primitives::TxEnv;
use reth::api::{FullNodeTypes, NodeTypesWithEngine};
use reth::builder::BuilderContext;
use reth::builder::components::ExecutorBuilder;
use reth::chainspec::EthereumHardforks;
use reth::primitives::EthPrimitives;
use reth::primitives::Receipt;
use reth::{
    api::ConfigureEvm,
    providers::BlockExecutionResult,
    revm::db::{State, states::bundle_state::BundleRetention},
};
use reth_chainspec::{ChainSpec, EthChainSpec};
use reth_evm::{
    OnStateHook,
    execute::{BlockExecutor, BlockExecutorProvider, Executor},
};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::{NodePrimitives, RecoveredBlock};
use std::fmt::Debug;
use std::num::NonZeroUsize;

pub struct BlockParallelExecutorProvider {
    strategy_factory: EthEvmConfig,
}

impl BlockParallelExecutorProvider {
    pub const fn new(strategy_factory: EthEvmConfig) -> Self {
        Self { strategy_factory }
    }
}

impl Clone for BlockParallelExecutorProvider {
    fn clone(&self) -> Self {
        Self {
            strategy_factory: self.strategy_factory.clone(),
        }
    }
}

impl BlockExecutorProvider for BlockParallelExecutorProvider {
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;

    type Executor<DB: Database> = ParallelExecutor<DB>;

    fn executor<DB>(&self, db: DB) -> Self::Executor<DB>
    where
        DB: Database,
    {
        let state_db = State::builder()
            .with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build();
        ParallelExecutor::new(self.strategy_factory.clone(), state_db)
    }
}

pub struct ParallelExecutor<DB> {
    strategy_factory: EthEvmConfig,
    db: State<DB>,
}

impl<DB> ParallelExecutor<DB> {
    pub fn new(strategy_factory: EthEvmConfig, db: State<DB>) -> Self {
        Self {
            strategy_factory,
            db,
        }
    }
}

impl<DB> Executor<DB> for ParallelExecutor<DB>
where
    DB: Database,
{
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;
    type Error = BlockExecutionError;

    fn execute_one(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<
        BlockExecutionResult<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Receipt>,
        Self::Error,
    > {
        // note: 1.receipt builder, 2.gas used calculated, 3. state write, 4. db persist
        // execute system contract call of `EIP-2935` and `EIP-4788`
        {
            let mut strategy = self
                .strategy_factory
                .executor_for_block(&mut self.db, block);
            strategy.apply_pre_execution_changes()?;
        }

        // execute block transactions parallel
        let parallel_execute_result = self.execute_block(block)?;

        // calculate requests
        let receipts = parallel_execute_result.receipts;
        let requests = self.calc_requests(receipts.clone(), block)?;

        // governance reward for full block, ommers...
        self.post_execution(block)?;

        // assemble new block execution result, there is no dump receipt
        let results = BlockExecutionResult {
            receipts,
            requests,
            gas_used: parallel_execute_result.gas_used,
        };
        self.db.merge_transitions(BundleRetention::Reverts);

        Ok(results)
    }

    fn execute_one_with_state_hook<F>(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
        _state_hook: F,
    ) -> Result<
        BlockExecutionResult<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Receipt>,
        Self::Error,
    >
    where
        F: OnStateHook + 'static,
    {
        // only strategy.execute_transaction used the Hook
        self.execute_one(block)
    }

    fn into_state(self) -> State<DB> {
        self.db
    }

    fn size_hint(&self) -> usize {
        self.db.bundle_state.size_hint()
    }
}

impl<DB> ParallelExecutor<DB>
where
    DB: Database,
{
    fn execute_block(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {
        let mut executor = metis_pe::ParallelExecutor::default();
        let env = self.strategy_factory.evm_env(block.header());
        let chain_spec = self.strategy_factory.chain_spec();
        let spec_id = *env.spec_id();
        let block_env = env.block_env;

        let tx_envs = block
            .transactions_recovered()
            .map(|recover_tx| recover_tx.into_tx_env())
            .collect::<Vec<TxEnv>>();

        let chain_id = chain_spec.chain_id();
        let chain = metis_pe::chain::Ethereum::custom(chain_id);
        let results = executor.execute_revm_parallel(
            &chain,
            StateStorageAdapter::new(&mut self.db),
            spec_id,
            block_env,
            tx_envs,
            NonZeroUsize::new(num_cpus::get()).unwrap_or(NonZeroUsize::new(1).unwrap()),
        );

        // todo(fk): total gas used should be the last `cumulative_gas_used`???
        let mut total_gas_used: u64 = 0;
        let receipts = results
            .unwrap()
            .into_iter()
            .map(|r| {
                total_gas_used += &r.receipt.cumulative_gas_used;
                crate::utils::from_receipt(r.tx_type, r.receipt)
            })
            .collect();

        Ok(BlockExecutionResult {
            receipts,
            gas_used: total_gas_used,
            ..Default::default()
        })
    }

    fn post_execution(
        &mut self,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<(), BlockExecutionError> {
        let env = self.strategy_factory.evm_env(block.header());
        let chain_spec = self.strategy_factory.chain_spec();
        let block_env = env.block_env;
        let body = block.body();
        let ommers = &body.ommers;
        let withdraws = &body.clone().withdrawals.unwrap();
        let mut balance_increments =
            post_block_balance_increments(chain_spec, &block_env, ommers, Some(withdraws));

        // Irregular state change at Ethereum DAO hardfork
        if chain_spec
            .fork(EthereumHardfork::Dao)
            .transitions_at_block(block.number)
        {
            // drain balances from hardcoded addresses.
            let drained_balance: u128 = self
                .db
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
        self.db
            .increment_balances(balance_increments)
            .map_err(|_| BlockValidationError::IncrementBalanceFailed)?;

        Ok(())
    }

    fn calc_requests(
        &mut self,
        receipts: Vec<Receipt>,
        block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
    ) -> Result<Requests, BlockExecutionError> {
        let spec = self.strategy_factory.chain_spec();
        let data: Requests = if spec.is_prague_active_at_timestamp(block.timestamp) {
            // Collect all EIP-6110 deposits
            let deposit_requests = eip6110::parse_deposits_from_receipts(&spec, &receipts)?;

            let mut requests = Requests::default();

            if !deposit_requests.is_empty() {
                requests.push_request_with_type(eip6110::DEPOSIT_REQUEST_TYPE, deposit_requests);
            }

            let mut strategy = self
                .strategy_factory
                .executor_for_block(&mut self.db, block);
            let evm = strategy.evm_mut();
            let mut system_caller = SystemCaller::new(spec);
            requests.extend(system_caller.apply_post_execution_changes(evm)?);
            requests
        } else {
            Requests::default()
        };

        Ok(data)
    }
}

#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl<Types, Node> ExecutorBuilder<Node> for ParallelExecutorBuilder
where
    Types: NodeTypesWithEngine<ChainSpec = ChainSpec, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
{
    type EVM = EthEvmConfig;
    type Executor = BlockParallelExecutorProvider;

    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        let evm_config = EthEvmConfig::new(ctx.chain_spec());
        let executor = BlockParallelExecutorProvider::new(evm_config.clone());

        Ok((evm_config, executor))
    }
}
