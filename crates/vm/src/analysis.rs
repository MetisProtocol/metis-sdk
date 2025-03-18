use metis_primitives::{Address, B256, U256};
use rustc_hash::FxHashMap as HashMap;
use smallvec::SmallVec;

#[derive(Debug, Default)]
pub struct ReadWriteSet {
    pub read: ReadSet,
    pub write: WriteSet,
}

pub type TxIdx = usize;
pub type TxIncarnation = usize;

// The origin of an account read. It could be from the live multi-version
// data structure or from storage (chain state before block execution).
#[derive(Debug, PartialEq)]
pub enum ReadOrigin {
    MvMemory(TxVersion),
    Storage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TxVersion {
    tx_idx: TxIdx,
    tx_incarnation: TxIncarnation,
}

/// Most locations only have one read origin. Lazy updated ones like
/// the beneficiary balance, raw transfer senders & recipients, etc. have a
/// list of lazy updates all the way to the first strict/absolute value.
pub type ReadOrigins = SmallVec<[ReadOrigin; 1]>;

/// A list of read origins.
pub type ReadSet = HashMap<Location, ReadOrigins>;

/// The updates made by this transaction incarnation, which is applied
/// to the multi-version data structure at the end of execution.
pub type WriteSet = Vec<(Location, Value)>;

/// Read or write account location.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Location {
    /// Read or write basic account's balance and nonce.
    Account(Address),
    /// Code hash address for CODE_HASH ops.
    CodeHash(Address),
    /// Strorage address and key for SLOAD and STORE ops.
    Storage(Address, U256),
}

/// Read or write value kinds in the transaction.
#[derive(Debug, Clone)]
pub enum Value {
    /// Read or write account values including nonce and balance.
    Account(AccountValue),
    /// Read the code hash in the contract bytecode.
    CodeHash(B256),
    /// Read or write the storage slot in the contract bytecode.
    Storage(U256),
    /// The account was self-destructed.
    SelfDestructed,
    /// Lazily update the beneficiary balance to avoid continuous
    /// dependencies as all transactions read and write to it.
    LazyRecipient(U256),
    /// Explicit balance subtraction & implicit nonce increment.
    LazySender(U256),
}

/// Basic value of an account
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountValue {
    /// The balance of the account.
    pub balance: U256,
    /// The nonce of the account.
    pub nonce: u64,
}
