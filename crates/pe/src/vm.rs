use crate::{
    AccountMeta, Entry, FinishExecFlags, Location, LocationHash, LocationValue, ReadOrigin,
    ReadOrigins, ReadSet, TxIdx, TxVersion, WriteSet,
    mv_memory::MvMemory,
    result::{ReadError, TxExecutionResult, VmExecutionError, VmExecutionResult},
};
use alloy_evm::EvmEnv;
use alloy_primitives::TxKind;
use metis_primitives::{
    BuildIdentityHasher, EvmState, HashMap, I257, ResultAndState, hash_deterministic,
};
#[cfg(feature = "compiler")]
use metis_vm::ExtCompileWorker;
use revm::context::ContextSetters;
#[cfg(feature = "compiler")]
use revm::handler::FrameInitOrResult;
use revm::handler::{EvmTr, FrameResult, FrameTr};
use revm::{
    Database, DatabaseRef, ExecuteEvm, MainBuilder, MainnetEvm,
    bytecode::Bytecode,
    context::{
        ContextTr, TxEnv,
        result::{EVMError, InvalidTransaction},
    },
    context_interface::{JournalTr, Transaction as _},
    handler::MainnetContext,
    primitives::{Address, B256, KECCAK_EMPTY, U256, hardfork::SpecId},
    state::{Account, AccountInfo, AccountStatus},
};
use revm::{handler::Handler, interpreter::interpreter_action::FrameInit};
use smallvec::SmallVec;
#[cfg(feature = "compiler")]
use std::sync::Arc;

// A database interface that intercepts reads while executing a specific
// transaction. It provides values from the multi-version data structure
// and storage, and tracks the read set of the current execution.
struct VmDB<'a, DB: DatabaseRef> {
    vm: &'a Vm<'a, DB>,
    tx_idx: TxIdx,
    tx: &'a TxEnv,
    from_hash: LocationHash,
    to_hash: Option<LocationHash>,
    to_code_hash: Option<B256>,
    // Indicates if we lazy update this transaction.
    // Only applied to raw transfers' senders & recipients at the moment.
    is_lazy: bool,
    read_set: ReadSet,
    read_accounts: HashMap<LocationHash, AccountMeta, BuildIdentityHasher>,
}

impl<'a, DB: DatabaseRef> VmDB<'a, DB> {
    fn new(
        vm: &'a Vm<'a, DB>,
        tx_idx: TxIdx,
        tx: &'a TxEnv,
        from_hash: LocationHash,
        to_hash: Option<LocationHash>,
    ) -> Result<Self, ReadError> {
        let mut db = Self {
            vm,
            tx_idx,
            tx,
            from_hash,
            to_hash,
            to_code_hash: None,
            is_lazy: false,
            // Unless it is a raw transfer that is lazy updated, we'll
            // read at least from the sender and recipient accounts.
            read_set: ReadSet::with_capacity_and_hasher(2, BuildIdentityHasher::default()),
            read_accounts: HashMap::with_capacity_and_hasher(2, BuildIdentityHasher::default()),
        };
        // We only lazy update raw transfers that already have the sender
        // or recipient in [`MvMemory`] since sequentially evaluating memory
        // locations with only one entry is much costlier than fully
        // evaluating it concurrently.
        if let TxKind::Call(to) = tx.kind {
            db.to_code_hash = db.get_code_hash(to)?;
            db.is_lazy = db.to_code_hash.is_none()
                && (vm.mv_memory.data.contains_key(&from_hash)
                    || vm.mv_memory.data.contains_key(&to_hash.unwrap()));
        }
        Ok(db)
    }

    fn hash_basic(&self, address: &Address) -> LocationHash {
        if address == &self.tx.caller {
            return self.from_hash;
        }
        if let TxKind::Call(to) = &self.tx.kind
            && to == address
        {
            return self.to_hash.unwrap();
        }
        hash_deterministic(Location::Basic(*address))
    }

    // Push a new read origin. Return an error when there's already
    // an origin but doesn't match the new one to force re-execution.
    fn push_origin(read_origins: &mut ReadOrigins, origin: ReadOrigin) -> Result<(), ReadError> {
        if let Some(prev_origin) = read_origins.last() {
            if prev_origin != &origin {
                return Err(ReadError::InconsistentRead);
            }
        } else {
            read_origins.push(origin);
        }
        Ok(())
    }

    fn get_code_hash(&mut self, address: Address) -> Result<Option<B256>, ReadError> {
        let location_hash = hash_deterministic(Location::CodeHash(address));
        let read_origins = self.read_set.entry(location_hash).or_default();

        // Try to read the latest code hash in [`MvMemory`]
        if let Some(written_txs) = self.vm.mv_memory.data.get(&location_hash)
            && let Some((tx_idx, Entry::Data(tx_incarnation, value))) =
                written_txs.range(..self.tx_idx).next_back()
        {
            match value {
                LocationValue::SelfDestructed => {
                    return Err(ReadError::SelfDestructedAccount);
                }
                LocationValue::CodeHash(code_hash) => {
                    Self::push_origin(
                        read_origins,
                        ReadOrigin::MvMemory(TxVersion {
                            tx_idx: *tx_idx,
                            tx_incarnation: *tx_incarnation,
                        }),
                    )?;
                    return Ok(Some(*code_hash));
                }
                _ => {}
            }
        };

        // Fallback to storage
        Self::push_origin(read_origins, ReadOrigin::Storage)?;
        Ok(self
            .vm
            .db
            .basic_ref(address)
            .map_err(|err| ReadError::StorageError(err.to_string()))?
            .map(|a| a.code_hash))
    }
}

impl<DB: DatabaseRef> Database for VmDB<'_, DB> {
    type Error = ReadError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let location_hash = self.hash_basic(&address);

        // We return a mock for non-contract addresses (for lazy updates) to avoid
        // unnecessarily evaluating its balance here.
        if self.is_lazy {
            if location_hash == self.from_hash {
                return Ok(Some(AccountInfo {
                    nonce: self.tx.nonce,
                    balance: U256::MAX,
                    code: Some(Bytecode::default()),
                    code_hash: KECCAK_EMPTY,
                }));
            } else if Some(location_hash) == self.to_hash {
                return Ok(None);
            }
        }

        let read_origins = self.read_set.entry(location_hash).or_default();
        let has_prev_origins = !read_origins.is_empty();
        // We accumulate new origins to either:
        // - match with the previous origins to check consistency
        // - register origins on the first read
        let mut new_origins = SmallVec::new();

        let mut final_account = None;
        // Note we use the negative zero for the fast check (balance_addition > 0)
        let mut balance_addition = I257::NEGATIVE_ZERO;
        let mut nonce_addition = 0;

        // Try reading from multi-version data
        if self.tx_idx > 0
            && let Some(written_txs) = self.vm.mv_memory.data.get(&location_hash)
        {
            let mut iter = written_txs.range(..self.tx_idx);

            // Fully evaluate lazy updates
            loop {
                match iter.next_back() {
                    Some((blocking_idx, Entry::Estimate)) => {
                        return Err(ReadError::Blocking(*blocking_idx));
                    }
                    Some((closest_idx, Entry::Data(tx_incarnation, value))) => {
                        // About to push a new origin
                        // Inconsistent: new origin will be longer than the previous!
                        if has_prev_origins && read_origins.len() == new_origins.len() {
                            return Err(ReadError::InconsistentRead);
                        }
                        let origin = ReadOrigin::MvMemory(TxVersion {
                            tx_idx: *closest_idx,
                            tx_incarnation: *tx_incarnation,
                        });
                        // Inconsistent: new origin is different from the previous!
                        if has_prev_origins
                            && unsafe { read_origins.get_unchecked(new_origins.len()) } != &origin
                        {
                            return Err(ReadError::InconsistentRead);
                        }
                        new_origins.push(origin);
                        match value {
                            LocationValue::Basic(basic) => {
                                final_account = Some(basic.clone());
                                break;
                            }
                            LocationValue::LazyRecipient(addition) => {
                                balance_addition += (*addition).into()
                            }
                            LocationValue::LazySender(subtraction) => {
                                balance_addition -= (*subtraction).into();
                                nonce_addition += 1;
                            }
                            _ => return Err(ReadError::InvalidValueType),
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }

        // Fall back to storage
        if final_account.is_none() {
            // Populate [Storage] on the first read
            if !has_prev_origins {
                new_origins.push(ReadOrigin::Storage);
            }
            // Inconsistent: previous origin is longer or didn't read
            // from storage for the last origin.
            else if read_origins.len() != new_origins.len() + 1
                || read_origins.last() != Some(&ReadOrigin::Storage)
            {
                return Err(ReadError::InconsistentRead);
            }
            final_account = match self.vm.db.basic_ref(address) {
                Ok(Some(basic)) => Some(basic),
                Ok(None) => (balance_addition.sign() > 0).then_some(AccountInfo::default()),
                Err(err) => return Err(ReadError::StorageError(err.to_string())),
            };
        }

        // Populate read origins on the first read.
        // Otherwise [read_origins] matches [new_origins] already.
        if !has_prev_origins {
            *read_origins = new_origins;
        }

        if let Some(mut account) = final_account {
            // Check sender nonce
            account.nonce += nonce_addition;
            if location_hash == self.from_hash && self.tx.nonce != account.nonce {
                return if self.tx_idx > 0 {
                    // For nonce too low error, only block the preceding transactions and
                    // cannot make the nonce of the account the same as tx.nonce, so return
                    // an error here.
                    if self.tx.nonce < account.nonce {
                        Err(ReadError::NonceTooLow {
                            tx: self.tx.nonce,
                            state: account.nonce,
                        })
                    } else {
                        Err(ReadError::Blocking(self.tx_idx - 1))
                    }
                } else if self.tx.nonce < account.nonce {
                    Err(ReadError::NonceTooLow {
                        tx: self.tx.nonce,
                        state: account.nonce,
                    })
                } else {
                    Err(ReadError::NonceTooHigh {
                        tx: self.tx.nonce,
                        state: account.nonce,
                    })
                };
            }

            // Fully evaluate the account and register it to read cache
            // to later check if they have changed (been written to).
            account.balance = (balance_addition + account.balance.into()).abs_value();

            let code_hash = if Some(location_hash) == self.to_hash {
                self.to_code_hash
            } else {
                self.get_code_hash(address)?
            };
            let code = if let Some(code_hash) = &code_hash {
                if let Some(code) = self.vm.mv_memory.new_bytecodes.get(code_hash) {
                    code.clone()
                } else {
                    match self.vm.db.code_by_hash_ref(*code_hash) {
                        Ok(code) => code,
                        Err(err) => return Err(ReadError::StorageError(err.to_string())),
                    }
                }
            } else {
                Bytecode::default()
            };
            let account_meta = if code_hash.is_none() {
                AccountMeta::EOA((account.balance, account.nonce))
            } else {
                AccountMeta::CA((account.balance, account.nonce))
            };
            self.read_accounts.insert(location_hash, account_meta);

            return Ok(Some(AccountInfo {
                balance: account.balance,
                nonce: account.nonce,
                code_hash: code_hash.unwrap_or(KECCAK_EMPTY),
                code: Some(code),
            }));
        }

        Ok(None)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.vm
            .db
            .code_by_hash_ref(code_hash)
            .map_err(|err| ReadError::StorageError(err.to_string()))
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let location_hash = hash_deterministic(Location::Storage(address, index));

        let read_origins = self.read_set.entry(location_hash).or_default();

        // Try reading from multi-version data
        if self.tx_idx > 0
            && let Some(written_txs) = self.vm.mv_memory.data.get(&location_hash)
            && let Some((closest_idx, entry)) = written_txs.range(..self.tx_idx).next_back()
        {
            match entry {
                Entry::Data(tx_incarnation, LocationValue::Storage(value)) => {
                    Self::push_origin(
                        read_origins,
                        ReadOrigin::MvMemory(TxVersion {
                            tx_idx: *closest_idx,
                            tx_incarnation: *tx_incarnation,
                        }),
                    )?;
                    return Ok(*value);
                }
                Entry::Estimate => return Err(ReadError::Blocking(*closest_idx)),
                _ => return Err(ReadError::InvalidValueType),
            }
        }

        // Fall back to storage
        Self::push_origin(read_origins, ReadOrigin::Storage)?;
        self.vm
            .db
            .storage_ref(address, index)
            .map_err(|err| ReadError::StorageError(err.to_string()))
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.vm
            .db
            .block_hash_ref(number)
            .map_err(|err| ReadError::StorageError(err.to_string()))
    }
}

pub(crate) struct Vm<'a, DB: DatabaseRef> {
    db: &'a DB,
    mv_memory: &'a MvMemory,
    evm_env: &'a EvmEnv,
    txs: &'a [TxEnv],
    // beneficiary_location_hash: LocationHash,
    // reward_policy: RewardPolicy,
    #[cfg(feature = "compiler")]
    worker: Arc<ExtCompileWorker>,
}

impl<'a, DB: DatabaseRef> Vm<'a, DB> {
    pub(crate) fn new(
        db: &'a DB,
        mv_memory: &'a MvMemory,
        evm_env: &'a EvmEnv,
        txs: &'a [TxEnv],
        #[cfg(feature = "compiler")] worker: Arc<ExtCompileWorker>,
    ) -> Self {
        Self {
            db,
            mv_memory,
            evm_env,
            txs,
            // beneficiary_location_hash: hash_deterministic(Location::Basic(
            //     evm_env.block_env.beneficiary,
            // )),
            // reward_policy: reward_policy(),
            #[cfg(feature = "compiler")]
            worker,
        }
    }

    // Execute a transaction. A successful execution returns:
    //   - A write-set consisting of locations and their updated values.
    //   - A read-set consisting of locations and their origins.
    //   - A transaction list affected by the current tx version.
    //
    // An execution may observe a read dependency on a lower transaction. This happens
    // when the last incarnation of the dependency wrote to a location that
    // this transaction reads, but it aborted before the read. In this case, the
    // dependency index is returned via [blocking_tx_idx]. An execution task for this
    // transaction is re-scheduled after the blocking dependency finishes its
    // next incarnation.
    //
    // When a transaction attempts to write a value to a location, the location and
    // value are added to the write set, possibly replacing a pair with a prior value
    // (if it is not the first time the transaction wrote to this location during the
    // execution).
    pub(crate) fn execute<HR>(
        &self,
        tx_version: &TxVersion,
    ) -> Result<VmExecutionResult<HR>, VmExecutionError>
    where
        HR: Clone
            + core::fmt::Debug
            + Send
            + Sync
            + 'static
            + core::cmp::Eq
            + core::convert::From<metis_primitives::HaltReason>,
    {
        // SAFETY: A correct scheduler would guarantee this index to be inbound.
        let tx = unsafe { self.txs.get_unchecked(tx_version.tx_idx) };
        let from_hash = hash_deterministic(Location::Basic(tx.caller));
        let to_hash = tx
            .kind
            .to()
            .map(|to| hash_deterministic(Location::Basic(*to)));

        // Execute
        let mut db = VmDB::new(self, tx_version.tx_idx, tx, from_hash, to_hash)
            .map_err(VmExecutionError::from)?;

        let mut evm = build_evm(&mut db, self.evm_env.clone());

        let result = {
            evm.set_tx(tx.clone());
            #[cfg(feature = "compiler")]
            let mut t = WithoutRewardBeneficiaryHandler::<_, HR>::new(self.worker.clone());
            #[cfg(not(feature = "compiler"))]
            let mut t = WithoutRewardBeneficiaryHandler::<_, HR>::default();
            t.run(&mut evm)
        };
        match result {
            Ok(result) => {
                // There are at least three locations most of the time: the sender,
                // the recipient, and the beneficiary accounts.
                let mut write_set =
                    WriteSet::with_capacity_and_hasher(100, BuildIdentityHasher::default());
                let mut state = evm.finalize();
                // IMPORTANT: `WithoutRewardBeneficiaryHandler` skips `reward_beneficiary`, so we must
                // explicitly apply the beneficiary (miner tip) reward into the finalized `state`.
                // Otherwise, proposers that commit pre-executed `ResultAndState` will produce a
                // different state root than validators that re-execute transactions.
                self.apply_beneficiary_reward_to_state(
                    &mut state,
                    tx_version.tx_idx,
                    tx,
                    result.gas_used(),
                );

                for (address, account) in &state {
                    if account.is_selfdestructed() {
                        // For now we are betting on [code_hash] triggering the sequential
                        // fallback when we read a self-destructed contract.
                        write_set.insert(
                            hash_deterministic(Location::CodeHash(*address)),
                            LocationValue::SelfDestructed,
                        );
                        continue;
                    }

                    if account.is_touched() {
                        let account_location_hash = hash_deterministic(Location::Basic(*address));

                        let read_account =
                            evm.db().read_accounts.get(&account_location_hash).cloned();

                        let has_code = !account.info.is_empty_code_hash();
                        let is_new_code = has_code
                            && read_account.is_none_or(|meta| matches!(meta, AccountMeta::EOA(_)));

                        // Write new account changes
                        if is_new_code
                            || account_location_hash == from_hash
                            || read_account.is_none()
                            // Nonce is changed or balance is changed.
                            || read_account.is_some_and(|meta| {
                                meta != AccountMeta::CA((account.info.balance, account.info.nonce)) && meta != AccountMeta::EOA((account.info.balance, account.info.nonce))
                            })
                        {
                            let is_lazy = evm.db().is_lazy;
                            if is_lazy {
                                if account_location_hash == from_hash {
                                    write_set.insert(
                                        account_location_hash,
                                        LocationValue::LazySender(U256::MAX - account.info.balance),
                                    );
                                } else if Some(account_location_hash) == to_hash {
                                    write_set.insert(
                                        account_location_hash,
                                        LocationValue::LazyRecipient(tx.value),
                                    );
                                }
                            }
                            // We don't register empty accounts after [SPURIOUS_DRAGON]
                            // as they are cleared. This can only happen via 2 ways:
                            // 1. Self-destruction which is handled by an if above.
                            // 2. Sending 0 ETH to an empty account, which we treat as a
                            // non-write here. A later read would trace back to storage
                            // and return a [None], i.e., [LoadedAsNotExisting]. Without
                            // this check it would write then read a [Some] default
                            // account, which may yield a wrong gas fee, etc.
                            else if !self
                                .evm_env
                                .cfg_env
                                .spec
                                .is_enabled_in(SpecId::SPURIOUS_DRAGON)
                                || !account.is_empty()
                            {
                                write_set.insert(
                                    account_location_hash,
                                    LocationValue::Basic(AccountInfo {
                                        balance: account.info.balance,
                                        nonce: account.info.nonce,
                                        code_hash: account.info.code_hash,
                                        code: account.info.code.clone(),
                                    }),
                                );
                            }
                        }

                        // Write new contract
                        if is_new_code {
                            write_set.insert(
                                hash_deterministic(Location::CodeHash(*address)),
                                LocationValue::CodeHash(account.info.code_hash),
                            );
                            self.mv_memory
                                .new_bytecodes
                                .entry(account.info.code_hash)
                                .or_insert_with(|| account.info.code.clone().unwrap_or_default());
                        }
                    }

                    for (slot, value) in account.changed_storage_slots() {
                        write_set.insert(
                            hash_deterministic(Location::Storage(*address, *slot)),
                            LocationValue::Storage(value.present_value),
                        );
                    }
                }

                // Drop the vm instance and the database instance.
                drop(evm);
                // Append lazy addresses when the mode is lazy.
                if db.is_lazy {
                    self.mv_memory.add_lazy_addresses([tx.caller]);
                    if let Some(to) = tx.kind.into_to() {
                        self.mv_memory.add_lazy_addresses([to]);
                    }
                }

                let mut flags = if tx_version.tx_idx > 0 && !db.is_lazy {
                    FinishExecFlags::NeedValidation
                } else {
                    FinishExecFlags::empty()
                };
                if self.mv_memory.record(tx_version, db.read_set, write_set) {
                    flags |= FinishExecFlags::WroteNewLocation;
                }
                let tx_type = reth_primitives::TxType::try_from(tx.tx_type).map_err(|err| {
                    VmExecutionError::ExecutionError(EVMError::Custom(err.to_string()))
                })?;

                // Combine result and state into ResultAndState to preserve all execution information
                let result_and_state: ResultAndState<HR> = ResultAndState { result, state };

                Ok(VmExecutionResult {
                    execution_result: TxExecutionResult::from_raw(tx_type, result_and_state),
                    flags,
                })
            }
            Err(EVMError::Database(read_error)) => Err(read_error.into()),
            Err(err) => {
                if tx_version.tx_idx > 0
                    && matches!(
                        err,
                        EVMError::Transaction(
                            InvalidTransaction::LackOfFundForMaxFee { .. }
                                | InvalidTransaction::NonceTooHigh { .. }
                        )
                    )
                {
                    Err(VmExecutionError::Blocking(tx_version.tx_idx - 1))
                } else {
                    Err(VmExecutionError::ExecutionError(err))
                }
            }
        }
    }

    /// Apply the per-transaction beneficiary reward (miner tip) directly into the finalized `state`.
    ///
    /// This mirrors the logic in `apply_rewards`, but updates the committed `EvmState` so that
    /// `ResultAndState` matches canonical EVM execution used during block validation.
    fn apply_beneficiary_reward_to_state(
        &self,
        state: &mut EvmState,
        tx_idx: TxIdx,
        tx: &TxEnv,
        gas_used: u64,
    ) {
        // Match the actual canonical execution in this node:
        // - Use effective gas price (1559/legacy aware)
        // - Credit the beneficiary with the full effective gas price.
        //
        // NOTE: Although Ethereum mainnet burns the basefee post-London, our witness diffs for this
        // network show that the canonical re-execution expects beneficiary balance increments of
        // (basefee + tip) * gas_used. To keep proposer payloads valid, we mirror that behavior here.
        let basefee = self.evm_env.block_env.basefee as u128;
        let effective_gas_price = tx.effective_gas_price(basefee);
        let coinbase_gas_price = effective_gas_price;

        // Log to make it easy to correlate witness diffs with reward math.
        tracing::debug!(
            target: "metis::parallel",
            beneficiary=?self.evm_env.block_env.beneficiary,
            spec=?self.evm_env.cfg_env.spec,
            basefee,
            effective_gas_price,
            coinbase_gas_price,
            gas_used,
            "beneficiary_reward: computed coinbase_gas_price (tip per gas)"
        );

        let amount = U256::from(coinbase_gas_price).saturating_mul(U256::from(gas_used));
        if amount.is_zero() {
            return;
        }

        let beneficiary: Address = self.evm_env.block_env.beneficiary;
        match state.get_mut(&beneficiary) {
            Some(account) => {
                account.info.balance = account.info.balance.saturating_add(amount);
                account.status = AccountStatus::Touched;
            }
            None => {
                // IMPORTANT (consensus-critical):
                // When a block has multiple txs, the beneficiary's balance at tx_k should include
                // rewards from tx_0..tx_{k-1}. If we start from the *parent* state every time, we'd
                // overwrite the in-block accumulated balance and diverge on multi-tx blocks.
                //
                // Therefore:
                // - Prefer the latest `< tx_idx` value from MvMemory (in-block state).
                // - Fall back to the underlying DB (parent state) if this is the first tx or the
                //   beneficiary hasn't been written yet.
                let beneficiary_location_hash = hash_deterministic(Location::Basic(beneficiary));
                let mut base_info: Option<AccountInfo> = None;

                if tx_idx > 0
                    && let Some(written_txs) = self.mv_memory.data.get(&beneficiary_location_hash)
                    && let Some((_, Entry::Data(_, value))) =
                        written_txs.range(..tx_idx).next_back()
                    && let LocationValue::Basic(info) = value
                {
                    base_info = Some(info.clone());
                }

                if base_info.is_none() {
                    base_info = match self.db.basic_ref(beneficiary) {
                        Ok(Some(info)) => Some(info),
                        Ok(None) => None,
                        Err(err) => {
                            tracing::warn!(
                                target: "metis::parallel",
                                beneficiary=?beneficiary,
                                %err,
                                "failed to read beneficiary account from db; defaulting balance to 0"
                            );
                            None
                        }
                    };
                }

                let mut info = base_info.unwrap_or_default();
                info.balance = info.balance.saturating_add(amount);
                state.insert(
                    beneficiary,
                    Account {
                        info,
                        status: AccountStatus::Touched,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[inline]
pub(crate) fn build_evm<DB: Database>(db: DB, evm_env: EvmEnv) -> MainnetEvm<MainnetContext<DB>> {
    // IMPORTANT (consensus-critical):
    // We observed blocks where beneficiary received `effective_gas_price * gas_used` (basefee was
    // NOT burned), which can only happen if the EVM's *handler/spec selection* is effectively
    // pre-London.
    //
    // Setting `CfgEnv.spec` alone is not sufficient if the underlying handler/spec was chosen at
    // `Context::new(.., spec_id)` time (used internally by revm mainnet handler). Therefore we
    // must construct the context with the correct spec id up front.
    let spec_id = evm_env.cfg_env.spec;

    // Construct the mainnet context with the correct internal spec selection.
    // Assigning to `MainnetContext<DB>` makes `BLOCK/TX/CFG/JOURNAL` concrete for the compiler.
    let ctx: MainnetContext<DB> = revm::Context::new(db, spec_id);

    ctx.with_cfg(evm_env.cfg_env)
        .with_block(evm_env.block_env)
        .build_mainnet()
}

pub struct WithoutRewardBeneficiaryHandler<EVM, HR> {
    _phantom: core::marker::PhantomData<EVM>,
    _halt: core::marker::PhantomData<HR>,
    #[cfg(feature = "compiler")]
    worker: Arc<metis_vm::ExtCompileWorker>,
}

impl<EVM, HR> Handler for WithoutRewardBeneficiaryHandler<EVM, HR>
where
    EVM: EvmTr<
            Context: ContextTr<Journal: JournalTr<State = EvmState>>,
            Frame: FrameTr<FrameInit = FrameInit, FrameResult = FrameResult>,
        >,
    // revm-handler requires `HaltReasonTr` which (for current revm-handler) implies:
    // - Eq
    // - From<revm::context_interface::result::HaltReason>
    HR: Clone
        + core::fmt::Debug
        + Send
        + Sync
        + 'static
        + core::cmp::Eq
        + core::convert::From<metis_primitives::HaltReason>,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>;
    type HaltReason = HR;

    fn reward_beneficiary(
        &self,
        _evm: &mut Self::Evm,
        _exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        // Skip beneficiary reward
        Ok(())
    }

    #[cfg(feature = "compiler")]
    fn frame_call(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
    ) -> Result<FrameInitOrResult<Self::Frame>, Self::Error> {
        let interpreter = &mut frame.interpreter;
        let code_hash = interpreter.bytecode.hash();
        let next_action = match code_hash {
            Some(code_hash) => {
                match self.worker.get_function(&code_hash) {
                    Ok(metis_vm::FetchedFnResult::NotFound) => {
                        use revm::context::Cfg;
                        // Compile the code
                        let spec_id = evm.ctx().cfg().spec().into();
                        let bytecode = interpreter.bytecode.bytes();
                        let _res = self.worker.spawn(spec_id, code_hash, bytecode);
                        evm.run_interpreter(interpreter)
                    }
                    Ok(metis_vm::FetchedFnResult::Found(_f)) => {
                        // TODO: sync revmc and revm structures for the compiler
                        // https://github.com/paradigmxyz/revmc/issues/75
                        // f.call_with_interpreter_and_memory(interpreter, memory, context)
                        evm.run_interpreter(interpreter)
                    }
                    Err(_) => {
                        // Fallback to the interpreter
                        evm.run_interpreter(interpreter)
                    }
                }
            }
            None => {
                // Fallback to the interpreter
                evm.run_interpreter(interpreter)
            }
        };
        frame.process_next_action(evm, next_action)
    }
}

#[cfg(not(feature = "compiler"))]
impl<EVM, HR> Default for WithoutRewardBeneficiaryHandler<EVM, HR> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            _halt: core::marker::PhantomData,
        }
    }
}

#[cfg(feature = "compiler")]
impl<EVM, HR> Default for WithoutRewardBeneficiaryHandler<EVM, HR> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            _halt: core::marker::PhantomData,
            worker: Arc::new(metis_vm::ExtCompileWorker::disable()),
        }
    }
}

#[cfg(feature = "compiler")]
impl<EVM, HR> WithoutRewardBeneficiaryHandler<EVM, HR> {
    #[inline]
    pub fn new(worker: Arc<metis_vm::ExtCompileWorker>) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            _halt: core::marker::PhantomData,
            worker,
        }
    }
}
