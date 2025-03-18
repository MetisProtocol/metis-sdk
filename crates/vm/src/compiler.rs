use std::sync::Arc;

use metis_primitives::{B256, SpecId};
use revm::{Database, handler::register::EvmHandler};
pub use revmc::llvm::Context as CompilerContext;
use revmc::{EvmCompiler, EvmCompilerFn, EvmLlvmBackend, OptimizationLevel};
use rustc_hash::FxHashMap as HashMap;

const DEFAULT_JIT_FUNCTION_NAME: &str = "test";

pub struct ExternalContext<'ctx> {
    _is_aot: bool,
    _context: &'ctx CompilerContext,
    compiler: EvmCompiler<EvmLlvmBackend<'ctx>>,
    // Note the jit_cache must be as longlive as the compiler with the execution engine.
    jit_cache: HashMap<B256, EvmCompilerFn>,
}

impl<'ctx> ExternalContext<'ctx> {
    pub fn new(context: &'ctx CompilerContext) -> Self {
        let is_aot = false;
        let backend = EvmLlvmBackend::new(context, is_aot, OptimizationLevel::Aggressive).unwrap();
        let compiler = EvmCompiler::new(backend);
        Self {
            _is_aot: is_aot,
            _context: context,
            compiler,
            jit_cache: Default::default(),
        }
    }
}

impl ExternalContext<'_> {
    fn get_function(
        &mut self,
        hash: B256,
        bytecode: &[u8],
        spec_id: SpecId,
    ) -> Option<EvmCompilerFn> {
        if let Some(f) = self.jit_cache.get(&hash) {
            Some(*f)
        } else {
            let f = unsafe {
                self.compiler
                    .jit(DEFAULT_JIT_FUNCTION_NAME, bytecode, spec_id)
                    .unwrap()
            };
            self.jit_cache.insert(hash, f);
            Some(f)
        }
    }
}

/// Register the compile handler into the VM.
pub fn register_compile_handler<DB: Database + 'static>(
    handler: &mut EvmHandler<'_, ExternalContext, DB>,
) {
    let prev = handler.execution.execute_frame.clone();
    handler.execution.execute_frame = Arc::new(move |frame, memory, tables, context| {
        let interpreter = frame.interpreter_mut();
        let bytecode_hash = interpreter.contract.hash.unwrap_or_default();
        if let Some(f) = context.external.get_function(
            bytecode_hash,
            interpreter.contract.bytecode.bytes_slice(),
            context.evm.spec_id(),
        ) {
            Ok(unsafe { f.call_with_interpreter_and_memory(interpreter, memory, context) })
        } else {
            prev(frame, memory, tables, context)
        }
    });
}
