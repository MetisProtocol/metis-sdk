use crate::runtime::get_runtime;
use alith::{Completion, Request, ResponseContent, ResponseTokenUsage, inference::LlamaEngine};
use metis_primitives::{Address, SpecId, address};
use revm::{
    context::{Cfg, ContextTr},
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{InputsImpl, InterpreterResult},
    precompile::{PrecompileError, PrecompileFn, PrecompileOutput, PrecompileResult, Precompiles},
};
use std::sync::OnceLock;
use tokio::sync::RwLock;

pub const INFERENCE_PRECOMPILE_ADDRESS: Address =
    address!("0x0000000000000000000000000000000000000999");
pub const DEFAULT_MODEL_PATH: &str = "/root/models/qwen2.5-1.5b-instruct-q5_k_m.gguf";
pub const GAS_PER_INFERENCE_TOKEN: u64 = 100;

fn get_or_init_engine() -> &'static RwLock<LlamaEngine> {
    static LLAMA_ENGINE: OnceLock<RwLock<LlamaEngine>> = OnceLock::new();
    LLAMA_ENGINE.get_or_init(|| {
        let model_path =
            std::env::var("METIS_VM_MODEL_PATH").unwrap_or(DEFAULT_MODEL_PATH.to_owned());

        let engine = get_runtime().block_on(async {
            LlamaEngine::new(model_path)
                .await
                .expect("Init inference engine failed")
        });
        RwLock::new(engine)
    })
}

fn inference_precompiles() -> &'static Precompiles {
    static INSTANCE: OnceLock<Precompiles> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut precompiles = Precompiles::prague().clone();
        // Custom inference precompile.
        precompiles.extend([(
            INFERENCE_PRECOMPILE_ADDRESS,
            |bytes, gas_limit| -> PrecompileResult {
                let prompt = String::from_utf8(bytes.to_vec())
                    .map_err(|_| PrecompileError::Fatal("Invalid UTF-8 input".to_string()))?;
                let request = Request::new(prompt, "".to_string());
                let result = get_runtime()
                    .block_on(async {
                        let mut engine = get_or_init_engine().write().await;
                        engine.completion(request).await
                    })
                    .map_err(|err| PrecompileError::Fatal(err.to_string()))?;
                let output = result.content();
                let gas_used = GAS_PER_INFERENCE_TOKEN * result.token_usage().total_tokens as u64;
                if gas_used > gas_limit {
                    Err(PrecompileError::OutOfGas)
                } else {
                    Ok(PrecompileOutput::new(gas_used, output.into()))
                }
            } as PrecompileFn,
        )
            .into()]);
        precompiles
    })
}

/// Eth precompiles with the inference precompile
#[derive(Clone)]
pub struct InferencePrecompiles {
    pub precompiles: EthPrecompiles,
}

impl InferencePrecompiles {
    pub fn new() -> Self {
        get_or_init_engine();
        Self {
            precompiles: EthPrecompiles::default(),
        }
    }
}

impl Default for InferencePrecompiles {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for InferencePrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        let spec_id = spec.clone().into();
        if spec_id == SpecId::PRAGUE {
            self.precompiles = EthPrecompiles {
                precompiles: inference_precompiles(),
                spec: spec.into(),
            }
        } else {
            PrecompileProvider::<CTX>::set_spec(&mut self.precompiles, spec);
        }
        true
    }

    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        inputs: &InputsImpl,
        is_static: bool,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        self.precompiles
            .run(context, address, inputs, is_static, gas_limit)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        self.precompiles.warm_addresses()
    }

    fn contains(&self, address: &Address) -> bool {
        self.precompiles.contains(address)
    }
}
