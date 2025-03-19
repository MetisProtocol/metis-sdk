pub use alloy_sol_types::{SolCall, sol};
pub use metis_primitives::hex;
use metis_primitives::{EVMBytecode, KECCAK_EMPTY, SpecId, TxKind, U256, address, keccak256};
use revm::{CacheState, Evm, primitives::AccountInfo};

sol! {
    contract Counter {
        function setNumber(uint256 newNumber) public;
        function increment() public;
    }
}

const COUNTER_BYTECODE: &[u8] = &hex!(
    "6080604052348015600e575f5ffd5b5060043610603a575f3560e01c80633fb5c1cb14603e5780638381f58a14604f578063d09de08a146068575b5f5ffd5b604d6049366004607d565b5f55565b005b60565f5481565b60405190815260200160405180910390f35b604d5f805490806076836093565b9190505550565b5f60208284031215608c575f5ffd5b5035919050565b5f6001820160af57634e487b7160e01b5f52601160045260245ffd5b506001019056fea2646970667358221220d02f44f09141ec64d16ec6c961c0852fa31422a8bc49e35d81afecaf8798d89364736f6c634300081c0033"
);

fn main() {
    // Prepare account and state
    let mut cache = CacheState::new(true);
    let caller = address!("1000000000000000000000000000000000000001");
    let to = address!("2000000000000000000000000000000000000002");
    let coinbase = address!("3000000000000000000000000000000000000003");
    let code_hash = keccak256(COUNTER_BYTECODE);
    let bytecode = EVMBytecode::new_raw(COUNTER_BYTECODE.into());
    cache.insert_account(
        caller,
        AccountInfo {
            balance: U256::from(1_000_000_000),
            code_hash: KECCAK_EMPTY,
            code: None,
            nonce: 0,
        },
    );
    cache.insert_account(
        to,
        AccountInfo {
            balance: U256::from(1_000_000_000),
            code_hash,
            code: Some(bytecode),
            nonce: 0,
        },
    );
    let mut state = revm::db::State::builder()
        .with_cached_prestate(cache)
        .with_bundle_update()
        .build();
    // New a VM and run the tx.
    let evm = Evm::builder()
        .with_db(&mut state)
        .with_spec_id(SpecId::CANCUN)
        .modify_cfg_env(|cfg| {
            cfg.chain_id = 1;
        })
        .modify_block_env(|block| {
            block.number = U256::from(30);
            block.gas_limit = U256::from(5_000_000);
            block.coinbase = coinbase;
        })
        .modify_tx_env(|tx| {
            tx.data = Counter::incrementCall {}.abi_encode().into();
            tx.gas_limit = 2_000_000;
            tx.gas_price = U256::from(1);
            tx.caller = caller;
            tx.transact_to = TxKind::Call(to);
            tx.nonce = Some(0);
        });
    let result = if std::env::var("METIS_VM_TRACING").is_ok() {
        let mut evm = evm
            .with_external_context(
                revm::inspectors::TracerEip3155::new(Box::new(std::io::stdout())).without_summary(),
            )
            .append_handler_register(revm::inspector_handle_register)
            .build();
        evm.transact().unwrap()
    } else {
        evm.build().transact().unwrap()
    };
    // Contract output, logs and state diff.
    // There are at least three locations most of the time: the sender,
    // the recipient, and the beneficiary accounts.
    println!("{result:?}");
}
