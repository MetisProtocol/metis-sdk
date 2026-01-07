use crate::{FinishExecFlags, TxIdx};
use alloy_consensus::TxType;
use metis_primitives::{
    DBErrorMarker, DatabaseRef, EVMError, EvmState, InvalidTransaction, TxNonce,
};
use op_revm::OpTransactionError;
use reth_primitives::Receipt;
use revm::context::result::ResultAndState as RevmResultAndState;

/// Database error definitions.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DBError {
    #[error("Storage error: {0}")]
    StorageNotFound(String),
}

impl DBErrorMarker for DBError {}

/// Errors when executing a block with the parallel executor.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParallelExecutorError {
    /// Nonce mismatch error including nonce too low and nonce too high errors.
    #[error("Nonce mismatch for tx #{tx_idx}. Expected {executed_nonce}, got {tx_nonce}")]
    NonceMismatch {
        /// Transaction index
        tx_idx: TxIdx,
        /// Nonce from tx (from the very input)
        tx_nonce: TxNonce,
        /// Nonce from state and execution
        executed_nonce: TxNonce,
    },
    #[error("Storage error: {0}")]
    StorageError(String),
    #[error("Execution error: {0}")]
    ExecutionError(
        #[source]
        #[from]
        ExecutionError,
    ),
    #[error("Unreachable error")]
    UnreachableError,
}

pub(crate) enum VmExecutionError {
    Retry,
    FallbackToSequential,
    Blocking(TxIdx),
    ExecutionError(ExecutionError),
}

/// Errors when reading a location.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// Cannot read location from storage.
    #[error("Failed reading from storage: {0}")]
    StorageError(String),
    /// This location has been written by a lower transaction.
    #[error("Read of location is blocked by tx #{0}")]
    Blocking(TxIdx),
    /// There has been an inconsistent read like reading the same
    /// location from storage in the first call but from [`Vm`] in
    /// the next.
    #[error("Inconsistent read")]
    InconsistentRead,
    /// Found an invalid nonce, like the first transaction of a sender
    /// not having a (+1) nonce from storage.
    #[error("Nonce {tx} too high, expected {state}")]
    NonceTooHigh { tx: u64, state: u64 },
    /// Found an invalid nonce, like the first transaction of a sender
    /// not having a (-1) nonce from storage.
    #[error("Nonce {tx} too low, expected {state}")]
    NonceTooLow { tx: u64, state: u64 },
    /// Read a self-destructed account that is very hard to handle, as
    /// there is no performant way to mark all storage slots as cleared.
    #[error("Tried to read self-destructed account")]
    SelfDestructedAccount,
    /// The stored value type doesn't match its location type.
    #[error("Invalid type of stored value")]
    InvalidValueType,
}

impl From<ReadError> for VmExecutionError {
    fn from(err: ReadError) -> Self {
        match err {
            ReadError::InconsistentRead => Self::Retry,
            ReadError::SelfDestructedAccount => Self::FallbackToSequential,
            ReadError::Blocking(tx_idx) => Self::Blocking(tx_idx),
            ReadError::NonceTooHigh { tx, state } => {
                Self::ExecutionError(EVMError::Transaction(InvalidTransaction::NonceTooHigh {
                    tx,
                    state,
                }))
            }
            ReadError::NonceTooLow { tx, state } => {
                Self::ExecutionError(EVMError::Transaction(InvalidTransaction::NonceTooLow {
                    tx,
                    state,
                }))
            }
            _ => Self::ExecutionError(EVMError::Database(err)),
        }
    }
}

impl DBErrorMarker for ReadError {}

/// The execution error from the underlying EVM error.
pub type ExecutionError = EVMError<ReadError>;

/// Execution result of a transaction
///
/// This structure contains the complete execution result including:
/// - receipt: The transaction receipt
/// - result_and_state: The complete revm ResultAndState preserving all execution details
///
/// CRITICAL: Using ResultAndState (instead of HashMap<Address, Account>) ensures
/// we don't lose storage slot changes, selfdestructs, created accounts, etc.
/// This prevents state root mismatch issues when used with reth's commit_transaction().
///
/// The generic parameter HR is the HaltReason type from the EVM implementation.
/// This allows metis-pe to return ResultAndState compatible with reth's executor.
#[derive(Debug, Clone)]
pub struct TxExecutionResult<HR = metis_primitives::HaltReason> {
    /// Receipt of the execution
    pub receipt: Receipt,
    /// Complete execution result and state from revm
    /// This preserves all information needed for correct state commitment
    pub result_and_state: RevmResultAndState<HR>,
}

impl<HR> TxExecutionResult<HR>
where
    HR: Clone + core::fmt::Debug,
{
    /// Construct an execution result from the raw result and state.
    ///
    /// This preserves the complete ResultAndState from revm, ensuring no information loss.
    #[inline]
    pub fn from_raw(
        tx_type: TxType,
        result_and_state: metis_primitives::ResultAndState<HR>,
    ) -> Self {
        let cumulative_gas_used = result_and_state.result.gas_used();
        let success = result_and_state.result.is_success();
        let logs = result_and_state.result.logs().to_vec();

        Self {
            receipt: Receipt {
                tx_type,
                success,
                cumulative_gas_used,
                logs,
            },
            result_and_state,
        }
    }

    /// Get a reference to the state for backward compatibility.
    ///
    /// DEPRECATED: This is temporary compatibility for old code paths.
    /// New code should use result_and_state directly with commit_transaction().
    #[inline]
    pub fn state(&self) -> &EvmState {
        &self.result_and_state.state
    }

    /// Get a mutable reference to the state for backward compatibility.
    ///
    /// DEPRECATED: This is temporary compatibility for old code paths.
    /// New code should use result_and_state directly with commit_transaction().
    /// WARNING: Mutating the state directly may cause inconsistencies!
    #[inline]
    pub fn state_mut(&mut self) -> &mut EvmState {
        &mut self.result_and_state.state
    }
}

pub(crate) struct VmExecutionResult<HR> {
    pub(crate) execution_result: TxExecutionResult<HR>,
    pub(crate) flags: FinishExecFlags,
}

/// Execution result of a block
pub type ParallelExecutorResult<HR> = Result<Vec<TxExecutionResult<HR>>, ParallelExecutorError>;

#[derive(Debug)]
pub(crate) enum AbortReason {
    FallbackToSequential,
    ExecutionError(ExecutionError),
}

#[inline]
pub(crate) fn evm_err_to_exec_error<DB: DatabaseRef>(
    err: EVMError<DB::Error>,
) -> ParallelExecutorError {
    match err {
        EVMError::Transaction(err) => ExecutionError::Transaction(err).into(),
        EVMError::Header(err) => ExecutionError::Header(err).into(),
        EVMError::Custom(err) => ExecutionError::Custom(err).into(),
        // Note that parallel execution requires recording the wrapper DB for read-write sets,
        // so DB errors of parallel and sequential executor are different. So we convert the
        // database error to the custom error.
        EVMError::Database(err) => ExecutionError::Custom(err.to_string()).into(),
    }
}

#[inline]
pub(crate) fn op_evm_err_to_exec_error<DB: DatabaseRef>(
    err: EVMError<DB::Error, OpTransactionError>,
) -> ParallelExecutorError {
    match err {
        EVMError::Transaction(err) => match err {
            OpTransactionError::Base(err) => ExecutionError::Transaction(err).into(),
            OpTransactionError::DepositSystemTxPostRegolith => {
                ExecutionError::Custom("DepositSystemTxPostRegolith".to_string()).into()
            }
            OpTransactionError::HaltedDepositPostRegolith => {
                ExecutionError::Custom("HaltedDepositPostRegolith".to_string()).into()
            }
            OpTransactionError::MissingEnvelopedTx => {
                ExecutionError::Custom("MissingEnvelopedTx".to_string()).into()
            }
        },
        EVMError::Header(err) => ExecutionError::Header(err).into(),
        EVMError::Custom(err) => ExecutionError::Custom(err).into(),
        EVMError::Database(err) => ExecutionError::Custom(err.to_string()).into(),
    }
}
