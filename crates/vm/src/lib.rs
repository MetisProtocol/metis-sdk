#[cfg(feature = "compiler")]
pub mod compiler;
#[cfg(feature = "compiler")]
pub use compiler::{CompilerContext, ExtCompileWorker, register_compile_handler};
#[cfg(feature = "compiler")]
pub mod pool;

#[cfg(feature = "inference")]
pub mod inference;
#[cfg(feature = "inference")]
pub use inference::{
    DEFAULT_MODEL_PATH, GAS_PER_INFERENCE_TOKEN, INFERENCE_PRECOMPILE_ADDRESS,
    register_inference_handler,
};

pub mod analysis;
pub mod env;
pub mod error;
mod runtime;

pub use error::Error;

#[cfg(test)]
mod tests;
