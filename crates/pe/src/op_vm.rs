use crate::{
    AccountMeta, Entry, FinishExecFlags, Location, LocationHash, LocationValue, ReadOrigin,
    ReadOrigins, ReadSet, TxIdx, TxVersion, WriteSet,
    mv_memory::MvMemory,
    mv_memory::{OpRewardPolicy, op_reward_policy},
    result::{ReadError, TxExecutionResult, VmExecutionError, VmExecutionResult},
    vm::WithoutRewardBeneficiaryHandler,
};
use alloy_evm::EvmEnv;
use alloy_primitives::TxKind;
use metis_primitives::{BuildIdentityHasher, HashMap, I257, Transaction, hash_deterministic};
#[cfg(feature = "compiler")]
use metis_vm::ExtCompileWorker;
use op_revm::{DefaultOp, OpBuilder, OpContext, OpEvm, OpSpecId, OpTransaction};
#[cfg(feature = "compiler")]
use revm::handler::FrameInitOrResult;
use revm::{
    Database, DatabaseRef, ExecuteEvm,
    bytecode::Bytecode,
    context::Cfg,
    context::{
        ContextTr, TxEnv,
        result::{EVMError, InvalidTransaction},
    },
    handler::EvmTr,
    handler::Handler,
    primitives::{Address, B256, KECCAK_EMPTY, U256, hardfork::SpecId},
    state::AccountInfo,
};
use smallvec::{SmallVec, smallvec};
#[cfg(feature = "compiler")]
use std::sync::Arc;

// A database interface that intercepts reads while executing a specific
// transaction. It provides values from the multi-version data structure
// and storage, and tracks the read set of the current execution.
struct OpVmDB<'a, DB: DatabaseRef> {
    vm: &'a OpVm<'a, DB>,
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

impl<'a, DB: DatabaseRef> OpVmDB<'a, DB> {
    fn new(
        vm: &'a OpVm<'a, DB>,
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
        if let TxKind::Call(to) = &self.tx.kind {
            if to == address {
                return self.to_hash.unwrap();
            }
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
        if let Some(written_txs) = self.vm.mv_memory.data.get(&location_hash) {
            if let Some((tx_idx, Entry::Data(tx_incarnation, value))) =
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
impl<DB: DatabaseRef> Database for OpVmDB<'_, DB> {
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
        if self.tx_idx > 0 {
            if let Some(written_txs) = self.vm.mv_memory.data.get(&location_hash) {
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
                                && unsafe { read_origins.get_unchecked(new_origins.len()) }
                                    != &origin
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
        if self.tx_idx > 0 {
            if let Some(written_txs) = self.vm.mv_memory.data.get(&location_hash) {
                if let Some((closest_idx, entry)) = written_txs.range(..self.tx_idx).next_back() {
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

pub(crate) struct OpVm<'a, DB: DatabaseRef> {
    db: &'a DB,
    mv_memory: &'a MvMemory,
    evm_env: &'a EvmEnv,
    txs: &'a [TxEnv],
    beneficiary_location_hash: LocationHash,
    op_reward_policy: OpRewardPolicy,
    #[cfg(feature = "compiler")]
    worker: Arc<ExtCompileWorker>,
}

impl<'a, DB: DatabaseRef> OpVm<'a, DB> {
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
            beneficiary_location_hash: hash_deterministic(Location::Basic(
                evm_env.block_env.beneficiary,
            )),
            op_reward_policy: op_reward_policy(),
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
    pub(crate) fn execute(
        &self,
        tx_version: &TxVersion,
    ) -> Result<VmExecutionResult, VmExecutionError> {
        // SAFETY: A correct scheduler would guarantee this index to be inbound.
        let tx = unsafe { self.txs.get_unchecked(tx_version.tx_idx) };
        let from_hash = hash_deterministic(Location::Basic(tx.caller));
        let to_hash = tx
            .kind
            .to()
            .map(|to| hash_deterministic(Location::Basic(*to)));

        // Execute
        let mut db = OpVmDB::new(self, tx_version.tx_idx, tx, from_hash, to_hash)
            .map_err(VmExecutionError::from)?;
        let mut evm = build_op_evm(&mut db, Default::default());

        let result_and_state = {
            evm.set_tx(OpTransaction::new(tx.clone()));
            #[cfg(feature = "compiler")]
            let mut t = WithoutRewardBeneficiaryHandler::new(self.worker.clone());
            #[cfg(not(feature = "compiler"))]
            let mut t = WithoutRewardBeneficiaryHandler::default();
            t.run(&mut evm)
        };
        match result_and_state {
            Ok(result_and_state) => {
                // There are at least three locations most of the time: the sender,
                // the recipient, and the beneficiary accounts.
                let mut write_set =
                    WriteSet::with_capacity_and_hasher(100, BuildIdentityHasher::default());
                for (address, account) in &result_and_state.state {
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

                        let read_account = evm
                            .0
                            .db()
                            .read_accounts
                            .get(&account_location_hash)
                            .cloned();

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
                            let is_lazy = evm.0.db().is_lazy;
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

                self.apply_rewards(
                    &mut write_set,
                    tx,
                    U256::from(result_and_state.result.gas_used()),
                    evm.ctx(),
                )?;

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

    // Apply rewards (balance increments) to beneficiary accounts, etc.
    fn apply_rewards<CtxDB: Database>(
        &self,
        write_set: &mut WriteSet,
        tx: &TxEnv,
        gas_used: U256,
        op_ctx: &mut op_revm::OpContext<CtxDB>,
    ) -> Result<(), VmExecutionError> {
        let mut gas_price = if let Some(priority_fee) = tx.gas_priority_fee {
            std::cmp::min(
                tx.gas_price,
                priority_fee.saturating_add(self.evm_env.block_env.basefee as u128),
            )
        } else {
            tx.gas_price
        };
        if self.evm_env.cfg_env.spec.is_enabled_in(SpecId::LONDON) {
            gas_price = gas_price.saturating_sub(self.evm_env.block_env.basefee as u128);
        }
        let gas_price = U256::from(gas_price);
        let rewards: SmallVec<[(LocationHash, U256); 1]> = match self.op_reward_policy {
            OpRewardPolicy::Optimism {
                l1_fee_recipient_location_hash,
                base_fee_vault_location_hash,
            } => {
                use op_revm::transaction::OpTxTr;
                use op_revm::transaction::deposit::DEPOSIT_TRANSACTION_TYPE;

                let is_deposit = op_ctx.tx().tx_type() == DEPOSIT_TRANSACTION_TYPE;
                if is_deposit {
                    SmallVec::new()
                } else {
                    let enveloped = op_ctx.tx().enveloped_tx().cloned();
                    let spec = op_ctx.cfg().spec();
                    let l1_block_info = op_ctx.chain();

                    let Some(enveloped_tx) = &enveloped else {
                        panic!("[OPTIMISM] Failed to load enveloped transaction.");
                    };
                    let l1_cost = l1_block_info.calculate_tx_l1_cost(enveloped_tx, spec);

                    smallvec![
                        (
                            self.beneficiary_location_hash,
                            gas_price.saturating_mul(gas_used)
                        ),
                        (l1_fee_recipient_location_hash, l1_cost),
                        (
                            base_fee_vault_location_hash,
                            U256::from(self.evm_env.block_env.basefee).saturating_mul(gas_used),
                        ),
                    ]
                }
            }
        };

        for (recipient, amount) in rewards {
            if let Some(value) = write_set.get_mut(&recipient) {
                match value {
                    LocationValue::Basic(basic) => {
                        basic.balance = basic.balance.saturating_add(amount)
                    }
                    LocationValue::LazySender(subtraction) => {
                        *subtraction = subtraction.saturating_sub(amount)
                    }
                    LocationValue::LazyRecipient(addition) => {
                        *addition = addition.saturating_add(amount)
                    }
                    _ => return Err(ReadError::InvalidValueType.into()),
                }
            } else {
                write_set.insert(recipient, LocationValue::LazyRecipient(amount));
            }
        }

        Ok(())
    }
}

#[inline]
pub(crate) fn build_op_evm<DB: Database>(db: DB, spec_id: OpSpecId) -> OpEvm<OpContext<DB>, ()> {
    use revm::Context;

    let op_ctx = Context::op().with_db(db).modify_cfg_chained(|cfg| {
        cfg.spec = spec_id;
    });
    op_ctx.build_op()
}
