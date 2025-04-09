use metis_primitives::{SpecId, TxKind};
use metis_vm::{INFERENCE_PRECOMPILE_ADDRESS, InferencePrecompiles};
use revm::{Context, ExecuteEvm, MainBuilder, MainContext};

fn main() {
    let mut evm = Context::mainnet()
        .modify_cfg_chained(|cfg| {
            cfg.spec = SpecId::PRAGUE;
        })
        .modify_tx_chained(|tx| {
            tx.data = String::from("Who are you?").as_bytes().to_vec().into();
            tx.gas_limit = 2_000_000;
            tx.kind = TxKind::Call(INFERENCE_PRECOMPILE_ADDRESS);
        })
        .build_mainnet()
        .with_precompiles(InferencePrecompiles::default());

    let result = evm.replay().unwrap();
    let result = result.result.output().unwrap_or_default();
    println!("{}", String::from_utf8(result.0.to_vec()).unwrap());
}
