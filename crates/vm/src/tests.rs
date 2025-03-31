#[allow(unused)]
use metis_primitives::{Bytes, EVMBytecode, FromHex};

#[cfg(feature = "compiler")]
#[tokio::test]
async fn contract_compile_test() {
    compile_test(
        Bytes::from_hex(include_str!("../../pe/tests/erc20/assets/ERC20Token.hex").trim()).unwrap(),
    )
    .await;
    compile_test(
        Bytes::from_hex(include_str!("../../pe/tests/uniswap/assets/SingleSwap.hex").trim())
            .unwrap(),
    )
    .await;
    compile_test(
        Bytes::from_hex(include_str!("../../pe/tests/uniswap/assets/SwapRouter.hex").trim())
            .unwrap(),
    )
    .await;

    compile_test(
        Bytes::from_hex(include_str!("../../pe/tests/uniswap/assets/UniswapV3Factory.hex").trim())
            .unwrap(),
    )
    .await;

    compile_test(
        Bytes::from_hex(include_str!("../../pe/tests/uniswap/assets/UniswapV3Pool.hex").trim())
            .unwrap(),
    )
    .await;

    compile_test(
        Bytes::from_hex(include_str!("../../pe/tests/uniswap/assets/WETH9.hex").trim()).unwrap(),
    )
    .await;
}

#[cfg(feature = "compiler")]
async fn compile_test(bytecode: Bytes) {
    use crate::{ExtCompileWorker, compiler::FetchedFnResult};
    use metis_primitives::{SpecId, keccak256};

    let code_hash = keccak256(&bytecode);
    let spec_id = SpecId::CANCUN;
    let compiler = ExtCompileWorker::new_aot().unwrap();
    println!("aS: {:?}", compiler.get_function(&code_hash));
    let r = compiler.block(spec_id, code_hash, bytecode).await.unwrap();
    assert!(matches!(r, FetchedFnResult::Found(_)))
}
