use std::sync::Arc;

use crate::runtime::get_runtime;
use alith::{
    Completion, Request, ResponseContent, core::chat::ResponseTokenUsage, inference::LlamaEngine,
};
use metis_primitives::{Address, Bytes, address};
use revm::{
    ContextPrecompile, ContextStatefulPrecompile, Database,
    handler::register::EvmHandler,
    primitives::{PrecompileErrors, PrecompileOutput, PrecompileResult},
};
use tokio::sync::RwLock;

pub const INFERENCE_PRECOMPILE_ADDRESS: Address =
    address!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
pub const DEFAULT_MODEL_PATH: &str = "/root/models/qwen2.5-1.5b-instruct-q5_k_m.gguf";
pub const GAS_PER_INFERENCE_TOKEN: u64 = 100;

struct InferencePrecompile<M: Completion + Send + Sync> {
    engine: RwLock<M>,
}

impl<M: Completion + Send + Sync, DB: Database> ContextStatefulPrecompile<DB>
    for InferencePrecompile<M>
{
    fn call(
        &self,
        bytes: &Bytes,
        _gas_limit: u64,
        _evmctx: &mut revm::InnerEvmContext<DB>,
    ) -> PrecompileResult {
        let prompt = String::from_utf8(bytes.to_vec()).map_err(|_| PrecompileErrors::Fatal {
            msg: "Invalid UTF-8 input".to_string(),
        })?;
        let request = Request::new(prompt, "".to_string());
        let result = get_runtime()
            .block_on(async {
                let mut engine = self.engine.write().await;
                engine.completion(request).await
            })
            .map_err(|err| PrecompileErrors::Fatal {
                msg: err.to_string(),
            })?;
        let output = result.content();
        let gas_used = GAS_PER_INFERENCE_TOKEN * result.token_usage().total_tokens as u64;
        Ok(PrecompileOutput::new(gas_used, output.into()))
    }
}

/// Register handler for external context to support the AI inference precompile function.
pub fn register_inference_handler<EXT, DB: Database>(
    handler: &mut EvmHandler<'_, EXT, DB>,
) {
    let precompiles = handler.pre_execution.load_precompiles();
    handler.pre_execution.load_precompiles = Arc::new(move || {
        let mut precompiles = precompiles.clone();
        let model_path =
            std::env::var("METIS_VM_MODEL_PATH").unwrap_or(DEFAULT_MODEL_PATH.to_owned());
        let engine = get_runtime().block_on(async {
            LlamaEngine::new(model_path)
                .await
                .expect("Init inference engine failed")
        });
        precompiles.extend([(
            INFERENCE_PRECOMPILE_ADDRESS,
            ContextPrecompile::ContextStateful(Arc::new(InferencePrecompile {
                engine: RwLock::new(engine),
            })),
        )]);
        precompiles
    });
}
