//! Parallel Ethereum payload builder implementation.
//!
//! This module provides a custom payload builder that executes transactions in parallel
//! using the Block-STM based parallel executor instead of the default serial execution.

use alloy_consensus::{Transaction, transaction::Recovered};
use alloy_evm::ToTxEnv;
use alloy_primitives::U256;
use alloy_rlp::Encodable;
use metis_primitives::TxEnv;
use reth_basic_payload_builder::{
    BuildArguments, BuildOutcome, MissingPayloadBehaviour, PayloadBuilder, PayloadConfig,
    is_better_payload,
};
use reth_chainspec::{ChainSpecProvider, EthChainSpec, EthereumHardforks};
use reth_consensus_common::validation::MAX_RLP_BLOCK_SIZE;
use reth_errors::ConsensusError;
use reth_ethereum_engine_primitives::{EthBuiltPayload, EthPayloadBuilderAttributes};
use reth_ethereum_payload_builder::EthereumBuilderConfig;
use reth_ethereum_primitives::{EthPrimitives, TransactionSigned};
use reth_evm::{
    ConfigureEvm, Evm, NextBlockEnvAttributes,
    execute::{BlockBuilder, BlockBuilderOutcome},
};
use reth_evm_ethereum::EthEvmConfig;
use reth_node_api::{FullNodeTypes, NodeTypes, PrimitivesTy, TxTy};
use reth_node_builder::{
    BuilderContext, PayloadTypes, components::PayloadBuilderBuilder as PayloadBuilderBuilderTrait,
};
use reth_node_core::cli::config::PayloadBuilderConfig;
use reth_payload_builder_primitives::PayloadBuilderError;
use reth_payload_primitives::PayloadBuilderAttributes;
use reth_revm::{database::StateProviderDatabase, db::State};
use reth_storage_api::StateProviderFactory;
use reth_transaction_pool::{
    BestTransactions, BestTransactionsAttributes, PoolTransaction, TransactionPool,
    ValidPoolTransaction,
    error::{Eip4844PoolTransactionError, InvalidPoolTransactionError},
};
use revm::Database;
use revm::context_interface::Block as _;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tracing::{debug, info, trace, warn};

use crate::state::StateStorageAdapter;

type BestTransactionsIter<Pool> = Box<
    dyn BestTransactions<Item = Arc<ValidPoolTransaction<<Pool as TransactionPool>::Transaction>>>,
>;

/// Simple error wrapper for parallel execution errors
#[derive(Debug)]
struct ParallelExecutionError(String);

impl std::fmt::Display for ParallelExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParallelExecutionError {}

/// Parallel Ethereum payload builder
///
/// This builder collects transactions and executes them in parallel using the Block-STM
/// parallel executor, instead of executing them one by one like the default builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelEthereumPayloadBuilder<Pool, Client, EvmConfig = EthEvmConfig> {
    /// Client providing access to node state.
    client: Client,
    /// Transaction pool.
    pool: Pool,
    /// The type responsible for creating the evm.
    evm_config: EvmConfig,
    /// Payload builder configuration.
    builder_config: EthereumBuilderConfig,
}

impl<Pool, Client, EvmConfig> ParallelEthereumPayloadBuilder<Pool, Client, EvmConfig> {
    /// `ParallelEthereumPayloadBuilder` constructor.
    pub const fn new(
        client: Client,
        pool: Pool,
        evm_config: EvmConfig,
        builder_config: EthereumBuilderConfig,
    ) -> Self {
        Self {
            client,
            pool,
            evm_config,
            builder_config,
        }
    }
}

// PayloadBuilder implementation for parallel execution
impl<Pool, Client, EvmConfig> PayloadBuilder
    for ParallelEthereumPayloadBuilder<Pool, Client, EvmConfig>
where
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
    <<EvmConfig as ConfigureEvm>::BlockExecutorFactory as reth_evm::block::BlockExecutorFactory>::EvmFactory:
        alloy_evm::EvmFactory<
            Spec = revm::primitives::hardfork::SpecId,
            BlockEnv = revm::context::BlockEnv,
        >,
    Client: StateProviderFactory + ChainSpecProvider<ChainSpec: EthereumHardforks> + Clone,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>,
{
    type Attributes = EthPayloadBuilderAttributes;
    type BuiltPayload = EthBuiltPayload;

    fn try_build(
        &self,
        args: BuildArguments<EthPayloadBuilderAttributes, EthBuiltPayload>,
    ) -> Result<BuildOutcome<EthBuiltPayload>, PayloadBuilderError> {
        parallel_ethereum_payload(
            self.evm_config.clone(),
            self.client.clone(),
            self.pool.clone(),
            self.builder_config.clone(),
            args,
            |attributes| self.pool.best_transactions_with_attributes(attributes),
        )
    }

    fn on_missing_payload(
        &self,
        _args: BuildArguments<Self::Attributes, Self::BuiltPayload>,
    ) -> MissingPayloadBehaviour<Self::BuiltPayload> {
        if self.builder_config.await_payload_on_missing {
            MissingPayloadBehaviour::AwaitInProgress
        } else {
            MissingPayloadBehaviour::RaceEmptyPayload
        }
    }

    fn build_empty_payload(
        &self,
        config: PayloadConfig<Self::Attributes>,
    ) -> Result<EthBuiltPayload, PayloadBuilderError> {
        let args = BuildArguments::new(Default::default(), config, Default::default(), None);

        parallel_ethereum_payload(
            self.evm_config.clone(),
            self.client.clone(),
            self.pool.clone(),
            self.builder_config.clone(),
            args,
            |attributes| self.pool.best_transactions_with_attributes(attributes),
        )?
        .into_payload()
        .ok_or_else(|| PayloadBuilderError::MissingPayload)
    }
}

/// Transaction collected for batch execution
#[derive(Debug)]
struct CollectedTransaction {
    /// The consensus transaction (recovered with signature)
    tx: Recovered<alloy_consensus::EthereumTxEnvelope<alloy_consensus::TxEip4844>>,
    /// Transaction hash
    tx_hash: alloy_primitives::TxHash,
    /// Gas used by this transaction (estimated)
    _gas_limit: u64,
    /// Base fee at the time
    _base_fee: u64,
    /// Blob gas used (if EIP-4844)
    _blob_gas_used: Option<u64>,
}

/// Constructs an Ethereum transaction payload using parallel execution.
///
/// This is the core function that implements the parallel execution strategy:
/// 1. Collect all valid transactions (same validation as Reth)
/// 2. Execute them in PARALLEL using execute_transactions() batch API
/// 3. Process results and build the payload
#[inline]
pub fn parallel_ethereum_payload<EvmConfig, Client, Pool, F>(
    evm_config: EvmConfig,
    client: Client,
    pool: Pool,
    builder_config: EthereumBuilderConfig,
    args: BuildArguments<EthPayloadBuilderAttributes, EthBuiltPayload>,
    best_txs: F,
) -> Result<BuildOutcome<EthBuiltPayload>, PayloadBuilderError>
where
    EvmConfig: ConfigureEvm<Primitives = EthPrimitives, NextBlockEnvCtx = NextBlockEnvAttributes>,
// metis-pe currently expects `alloy_evm::EvmEnv` with default generics
// (revm `SpecId` + revm `BlockEnv`). Constrain EVM config accordingly so we can pass
// the canonical `next_evm_env` directly without lossy conversions.
    <<EvmConfig as ConfigureEvm>::BlockExecutorFactory as reth_evm::block::BlockExecutorFactory>::EvmFactory:
        alloy_evm::EvmFactory<
            Spec = revm::primitives::hardfork::SpecId,
            BlockEnv = revm::context::BlockEnv,
        >,
    Client: StateProviderFactory + ChainSpecProvider<ChainSpec: EthereumHardforks>,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TransactionSigned>>,
    F: FnOnce(BestTransactionsAttributes) -> BestTransactionsIter<Pool>,
{
    let BuildArguments {
        mut cached_reads,
        config,
        cancel,
        best_payload,
    } = args;
    let PayloadConfig {
        parent_header,
        attributes,
    } = config;

    let state_provider = client.state_by_block_hash(parent_header.hash())?;
    let state = StateProviderDatabase::new(&state_provider);
    let mut db = State::builder()
        .with_database(cached_reads.as_db_mut(state))
        .with_bundle_update()
        .build();

    // Build NextBlockEnvAttributes once and reuse it for:
    // 1) builder_for_next_block (reth canonical builder env)
    // 2) evm_config.next_evm_env (reth canonical cfg+block env for execution)
    let next_attrs = NextBlockEnvAttributes {
        timestamp: attributes.timestamp(),
        suggested_fee_recipient: attributes.suggested_fee_recipient(),
        prev_randao: attributes.prev_randao(),
        gas_limit: builder_config.gas_limit(parent_header.gas_limit),
        parent_beacon_block_root: attributes.parent_beacon_block_root(),
        withdrawals: Some(attributes.withdrawals().clone()),
    };

    let mut builder = evm_config
        .builder_for_next_block(&mut db, &parent_header, next_attrs.clone())
        .map_err(PayloadBuilderError::other)?;

    let chain_spec = client.chain_spec();

    info!(target: "payload_builder", id=%attributes.id, parent_header = ?parent_header.hash(), parent_number = parent_header.number, "Building payload with parallel execution");

    let block_gas_limit: u64 = builder.evm_mut().block().gas_limit();
    let base_fee = builder.evm_mut().block().basefee();

    let mut best_txs = best_txs(BestTransactionsAttributes::new(
        base_fee,
        builder
            .evm_mut()
            .block()
            .blob_gasprice()
            .map(|gasprice| gasprice as u64),
    ));

    builder.apply_pre_execution_changes().map_err(|err| {
        warn!(target: "payload_builder", %err, "failed to apply pre-execution changes");
        PayloadBuilderError::Internal(err.into())
    })?;

    let blob_params = chain_spec.blob_params_at_timestamp(attributes.timestamp);
    let max_blob_count = blob_params
        .as_ref()
        .map(|params| params.max_blob_count)
        .unwrap_or_default();

    let is_osaka = chain_spec.is_osaka_active_at_timestamp(attributes.timestamp);

    // ====================================================================
    // STEP 1: Collect all valid transactions (same validation as Reth)
    // ====================================================================
    let mut collected_transactions = Vec::new();
    let mut cumulative_gas_for_validation = 0u64;
    let mut block_blob_count = 0u64;
    let mut block_transactions_rlp_length = 0usize;
    let mut blob_sidecars_to_include = Vec::new();

    debug!(target: "payload_builder", "Phase 1: Collecting and validating transactions");
    debug!(target: "payload_builder",
        "Collection params: block_gas_limit={}, max_blob_count={:?}, is_osaka={}",
        block_gas_limit, max_blob_count, is_osaka
    );

    while let Some(pool_tx) = best_txs.next() {
        // Check gas limit
        if cumulative_gas_for_validation + pool_tx.gas_limit() > block_gas_limit {
            best_txs.mark_invalid(
                &pool_tx,
                InvalidPoolTransactionError::ExceedsGasLimit(pool_tx.gas_limit(), block_gas_limit),
            );
            continue;
        }

        // Check if cancelled
        if cancel.is_cancelled() {
            return Ok(BuildOutcome::Cancelled);
        }

        // Convert to signed transaction
        let tx = pool_tx.to_consensus();

        // Check block size (Osaka)
        let estimated_block_size_with_tx = block_transactions_rlp_length
            + tx.inner().length()
            + attributes.withdrawals().length()
            + 1024;

        if is_osaka && estimated_block_size_with_tx > MAX_RLP_BLOCK_SIZE {
            best_txs.mark_invalid(
                &pool_tx,
                InvalidPoolTransactionError::OversizedData {
                    size: estimated_block_size_with_tx,
                    limit: MAX_RLP_BLOCK_SIZE,
                },
            );
            continue;
        }

        // Check blob limit
        let mut blob_sidecar_for_tx = None;
        if let Some(blob_tx) = tx.as_eip4844() {
            let tx_blob_count = blob_tx.tx().blob_versioned_hashes.len() as u64;

            if block_blob_count + tx_blob_count > max_blob_count {
                trace!(target: "payload_builder", tx=?tx.hash(), ?block_blob_count, "skipping blob transaction");
                best_txs.mark_invalid(
                    &pool_tx,
                    InvalidPoolTransactionError::Eip4844(
                        Eip4844PoolTransactionError::TooManyEip4844Blobs {
                            have: block_blob_count + tx_blob_count,
                            permitted: max_blob_count,
                        },
                    ),
                );
                continue;
            }

            let blob_sidecar_result = 'sidecar: {
                let Some(sidecar) = pool
                    .get_blob(*tx.hash())
                    .map_err(PayloadBuilderError::other)?
                else {
                    break 'sidecar Err(Eip4844PoolTransactionError::MissingEip4844BlobSidecar);
                };

                if is_osaka {
                    if sidecar.is_eip7594() {
                        Ok(sidecar)
                    } else {
                        Err(Eip4844PoolTransactionError::UnexpectedEip4844SidecarAfterOsaka)
                    }
                } else if sidecar.is_eip4844() {
                    Ok(sidecar)
                } else {
                    Err(Eip4844PoolTransactionError::UnexpectedEip7594SidecarBeforeOsaka)
                }
            };

            blob_sidecar_for_tx = match blob_sidecar_result {
                Ok(sidecar) => Some(sidecar),
                Err(error) => {
                    best_txs.mark_invalid(&pool_tx, InvalidPoolTransactionError::Eip4844(error));
                    continue;
                }
            };

            // Update blob count
            block_blob_count += tx_blob_count;

            // Skip further blobs if we've reached the limit
            if block_blob_count == max_blob_count {
                best_txs.skip_blobs();
            }
        }

        // Transaction passed all validation - collect it
        cumulative_gas_for_validation += pool_tx.gas_limit();
        block_transactions_rlp_length += tx.inner().length();

        let tx_hash = *tx.hash();

        // Calculate blob gas used for this transaction
        let tx_blob_gas_used = tx.blob_gas_used();

        collected_transactions.push(CollectedTransaction {
            tx: tx.clone(),
            tx_hash,
            _gas_limit: pool_tx.gas_limit(),
            _base_fee: base_fee,
            _blob_gas_used: tx_blob_gas_used,
        });

        // Store blob sidecar for later inclusion
        if let Some(sidecar) = blob_sidecar_for_tx {
            blob_sidecars_to_include.push(sidecar);
        }
    }

    debug!(target: "payload_builder",
        tx_count = collected_transactions.len(),
        "Phase 1 complete: collected {} transactions, total_gas_limit={}, blob_count={}",
        collected_transactions.len(),
        cumulative_gas_for_validation,
        blob_sidecars_to_include.len()
    );

    // ====================================================================
    // STEP 2: Execute transactions in parallel
    // 1. Execute all transactions in parallel using Block-STM
    // 2. Save parallel execution receipts
    // 3. Add transactions to block via builder (for proper block structure)
    // 4. Replace receipts with parallel execution results
    // ====================================================================
    let mut total_fees = U256::ZERO;

    if !collected_transactions.is_empty() {
        debug!(target: "payload_builder",
            "Phase 2: Starting parallel execution of {} transactions",
            collected_transactions.len()
        );

        let tx_count = collected_transactions.len();
        let parallel_start = std::time::Instant::now();

        // IMPORTANT:
        // Do NOT hand-roll cfg_env/spec_id/block_env. Must match reth's canonical next_evm_env,
        // otherwise proposer state_root will not match validator/other nodes.
        let evm_env = evm_config
            .next_evm_env(&parent_header, &next_attrs)
            .map_err(PayloadBuilderError::other)?;

        let block_number = parent_header.number + 1;
        // Ensure state clearing semantics match reth serial execution.
        // This affects empty-account clearing (EIP-161 / SpuriousDragon) and can change state root.
        let state_clear_flag = chain_spec.is_spurious_dragon_active_at_block(block_number);
        builder
            .evm_mut()
            .db_mut()
            .set_state_clear_flag(state_clear_flag);

        // Convert transactions to TxEnv format
        let tx_envs: Vec<TxEnv> = collected_transactions
            .iter()
            .map(|ct| ct.tx.to_tx_env())
            .collect();

        debug!(target: "payload_builder", "Step 2a: Executing {} transactions in parallel", tx_count);

        // Execute in parallel
        let num_threads = num_cpus::get();
        // IMPORTANT: align metis-pe ResultAndState<HR> with reth executor's concrete HaltReason type
        let mut parallel_executor = metis_pe::ParallelExecutor::<
            <<<EvmConfig as ConfigureEvm>::BlockExecutorFactory as reth_evm::block::BlockExecutorFactory>::EvmFactory as alloy_evm::EvmFactory>::HaltReason,
        >::default();

        let results = match parallel_executor.execute(
            StateStorageAdapter::new(builder.evm_mut().db_mut()),
            evm_env,
            tx_envs,
            NonZeroUsize::new(num_threads).unwrap(),
        ) {
            Ok(results) => results,
            Err(err) => {
                warn!(target: "payload_builder", "Parallel execution failed: {:?}", err);
                return Err(PayloadBuilderError::other(ParallelExecutionError(format!(
                    "{err:?}"
                ))));
            }
        };

        let parallel_duration = parallel_start.elapsed();
        debug!(target: "payload_builder",
            "Parallel execution stats: total_txs={}, duration={:?}, avg_per_tx={:?}us",
            tx_count,
            parallel_duration,
            parallel_duration.as_micros() / tx_count as u128
        );

        // Fee estimation and cumulative gas will be computed while committing pre-executed ResultAndState
        // in Step 2b, using the canonical gas_used returned by commit_transaction.
        total_fees = U256::ZERO;

        // Step 2b: Commit pre-executed transactions using reth's native commit_transaction
        //
        // IMPORTANT:
        // - Do NOT manually apply state (StateAccumulator / insert_account_with_storage).
        // - Do NOT cache/replace receipts.
        // The only correct way to keep proposer/validator roots identical is to feed the
        // precomputed ResultAndState into the executor's commit_transaction.
        debug!(target: "payload_builder",
            "Step 2b: Committing {} pre-executed transactions via commit_transaction()",
            tx_count
        );

        let commit_start = std::time::Instant::now();
        let mut committed_count = 0usize;

        // Consume txs/results in the same order.
        for (idx, (ct, result)) in collected_transactions
            .into_iter()
            .zip(results.into_iter())
            .enumerate()
        {
            let tx_hash = ct.tx_hash;

            // 1) Add transaction to block body (no execution)
            let tx = ct.tx;
            // Fee estimation (only used to compare payloads) must be computed before moving `tx`
            let miner_fee = tx
                .effective_tip_per_gas(base_fee)
                .expect("fee is always valid");
            builder.push_transaction_to_body(tx.clone())?;

            // 2) Commit the pre-executed ResultAndState using executor.commit_transaction()
            // IMPORTANT:
            // revm::db::State will panic if we commit accounts that are not present in its internal
            // cache ("All accounts should be present inside cache"). Prewarm the cache for every
            // touched address in this tx's state before committing.
            {
                let mut addrs: Vec<_> = result.result_and_state.state.keys().copied().collect();
                addrs.sort_unstable();
                let db = builder.evm_mut().db_mut();
                for addr in addrs {
                    db.basic(addr).map_err(PayloadBuilderError::other)?;
                }
            }
            match builder.commit_executed_transaction(result.result_and_state, tx) {
                Ok(gas_used) => {
                    committed_count += 1;

                    total_fees += alloy_primitives::U256::from(miner_fee)
                        * alloy_primitives::U256::from(gas_used);
                }
                Err(err) => {
                    warn!(target: "payload_builder",
                        "❌ TX[{}] commit failed for 0x{:x}: {:?}",
                        idx, tx_hash, err
                    );
                    return Err(PayloadBuilderError::other(err));
                }
            }
        }

        let commit_duration = commit_start.elapsed();
        debug!(target: "payload_builder",
            "Step 2b complete: committed {} txs in {:?} (avg {:?} per tx)",
            committed_count,
            commit_duration,
            commit_duration / (committed_count.max(1) as u32)
        );

        debug!(target: "payload_builder",
            "Execution stats: parallel_executed={}, committed={}, parallel_time={:?}, commit_time={:?}, total_time={:?}",
            tx_count, committed_count,
            parallel_duration, commit_duration, parallel_start.elapsed()
        );
    }

    debug!(target: "payload_builder",
        %total_fees,
        "Phase 2 complete: fees={}",
        total_fees
    );

    // ====================================================================
    // STEP 3: Check if we have a better block and finalize
    // ====================================================================
    if !is_better_payload(best_payload.as_ref(), total_fees) {
        drop(builder);
        return Ok(BuildOutcome::Aborted {
            fees: total_fees,
            cached_reads,
        });
    }

    // Call builder.finish() to apply post_execution (block rewards, withdrawals, etc.)
    // and calculate the final state root.
    // Note: We've already executed transactions in parallel and committed the state,
    // so builder.finish() will add block rewards on top of our transaction state.

    debug!(target: "payload_builder", "Finalizing block and applying post_execution");

    let BlockBuilderOutcome {
        execution_result,
        block,
        ..
    } = builder.finish(&state_provider)?;

    // Receipts are now correctly generated by commit_transaction during parallel execution
    debug!(target: "payload_builder",
        "Block finalized: {} receipts, gas_used={}",
        execution_result.receipts.len(),
        execution_result.gas_used
    );

    let requests = chain_spec
        .is_prague_active_at_timestamp(attributes.timestamp)
        .then_some(execution_result.requests);

    let sealed_block = Arc::new(block.sealed_block().clone());
    debug!(target: "payload_builder", id=%attributes.id, sealed_block_header = ?sealed_block.sealed_header(), "sealed built block");

    if is_osaka && sealed_block.rlp_length() > MAX_RLP_BLOCK_SIZE {
        return Err(PayloadBuilderError::other(ConsensusError::BlockTooLarge {
            rlp_length: sealed_block.rlp_length(),
            max_rlp_length: MAX_RLP_BLOCK_SIZE,
        }));
    }

    // Build payload with blob sidecars
    let payload = if blob_sidecars_to_include.is_empty() {
        EthBuiltPayload::new(attributes.id, sealed_block, total_fees, requests)
    } else {
        // Build the appropriate BlobSidecars type based on Osaka activation
        let blob_sidecars = if is_osaka {
            // EIP-7594 PeerDAS sidecars (Osaka+)
            let sidecars_7594: Vec<_> = blob_sidecars_to_include
                .iter()
                .filter_map(|s| s.as_eip7594().cloned())
                .collect();
            reth_payload_builder::BlobSidecars::eip7594(sidecars_7594)
        } else {
            // EIP-4844 blob sidecars (pre-Osaka)
            let sidecars_4844: Vec<_> = blob_sidecars_to_include
                .iter()
                .filter_map(|s| s.as_eip4844().cloned())
                .collect();
            reth_payload_builder::BlobSidecars::eip4844(sidecars_4844)
        };

        EthBuiltPayload::new(attributes.id, sealed_block, total_fees, requests)
            .with_sidecars(blob_sidecars)
    };

    Ok(BuildOutcome::Better {
        payload,
        cached_reads,
    })
}

// ====================================================================
// Integration with Reth's Component System
// ====================================================================

/// Builder for creating ParallelEthereumPayloadBuilder instances.
///
/// This struct integrates our parallel payload builder into Reth's component system.
#[derive(Clone, Default, Debug)]
#[non_exhaustive]
pub struct ParallelPayloadBuilderBuilder;

impl<Types, Node, Pool, Evm> PayloadBuilderBuilderTrait<Node, Pool, Evm>
    for ParallelPayloadBuilderBuilder
where
    Types: NodeTypes<ChainSpec: EthereumHardforks + EthChainSpec, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
    Pool: TransactionPool<Transaction: PoolTransaction<Consensus = TxTy<Node::Types>>>
        + Unpin
        + 'static,
    Evm: ConfigureEvm<Primitives = PrimitivesTy<Types>, NextBlockEnvCtx = NextBlockEnvAttributes>
        + 'static,
    <<Evm as ConfigureEvm>::BlockExecutorFactory as reth_evm::block::BlockExecutorFactory>::EvmFactory:
        alloy_evm::EvmFactory<
            Spec = revm::primitives::hardfork::SpecId,
            BlockEnv = revm::context::BlockEnv,
        >,
    Types::Payload: PayloadTypes<
            BuiltPayload = EthBuiltPayload,
            PayloadAttributes = reth_ethereum_engine_primitives::EthPayloadAttributes,
            PayloadBuilderAttributes = EthPayloadBuilderAttributes,
        >,
{
    type PayloadBuilder = ParallelEthereumPayloadBuilder<Pool, Node::Provider, Evm>;

    async fn build_payload_builder(
        self,
        ctx: &BuilderContext<Node>,
        pool: Pool,
        evm_config: Evm,
    ) -> eyre::Result<Self::PayloadBuilder> {
        let conf = ctx.payload_builder_config();
        let chain = ctx.chain_spec().chain();
        let gas_limit = conf.gas_limit_for(chain);

        Ok(ParallelEthereumPayloadBuilder::new(
            ctx.provider().clone(),
            pool,
            evm_config,
            EthereumBuilderConfig::new().with_gas_limit(gas_limit),
        ))
    }
}
