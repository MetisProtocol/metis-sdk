use crate::{
    env::{module_name, store_path},
    error::Error,
    pool::CompilePool,
};
use libloading::{Library, Symbol};
use lru::LruCache;
use metis_primitives::{B256, Bytes, SpecId};
use revm::{
    Database,
    context::{
        Cfg, JournalOutput,
        result::{EVMError, HaltReason, InvalidTransaction},
    },
    context_interface::{ContextTr, JournalTr},
    handler::{
        EthFrame, EvmTr, FrameInitOrResult, Handler, PrecompileProvider,
        instructions::InstructionProvider,
    },
    interpreter::{InterpreterResult, interpreter::EthInterpreter},
};
use revmc::EvmCompilerFn;
use revmc::{EvmCompiler, EvmLlvmBackend, OptimizationLevel, llvm::Context};
use rustc_hash::FxBuildHasher;
use rustc_hash::FxHashMap as HashMap;
use std::{fmt::Debug, fs, path::PathBuf};
use std::{
    mem::transmute,
    sync::{Arc, TryLockError},
};

pub use revmc::llvm::Context as InnerContext;

/// A Context is not thread safe and cannot be shared across threads.
/// Multiple Contexts can, however, execute on different threads simultaneously
/// according to the LLVM docs.
#[derive(Debug, PartialEq, Eq)]
pub struct CompilerContext(pub(crate) InnerContext);

unsafe impl Send for CompilerContext {}
unsafe impl Sync for CompilerContext {}

impl CompilerContext {
    pub fn create() -> Self {
        Self(InnerContext::create())
    }
}

/// EVM Compile flags.
#[derive(Debug)]
pub(crate) struct CompileOptions {
    pub is_aot: bool,
    pub opt_level: OptimizationLevel,
    pub no_gas: bool,
    pub no_len_checks: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            is_aot: false,
            opt_level: OptimizationLevel::Aggressive,
            no_gas: false,
            no_len_checks: false,
        }
    }
}

pub(crate) struct Compiler<'ctx> {
    pub opts: CompileOptions,
    compiler: EvmCompiler<EvmLlvmBackend<'ctx>>,
}

unsafe impl Send for Compiler<'_> {}
unsafe impl Sync for Compiler<'_> {}

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn new(context: &'ctx CompilerContext, opts: CompileOptions) -> Result<Self, Error> {
        let backend = EvmLlvmBackend::new(&context.0, opts.is_aot, OptimizationLevel::Aggressive)
            .map_err(|err| Error::BackendInit(err.to_string()))?;
        let mut compiler = EvmCompiler::new(backend);
        compiler.gas_metering(opts.no_gas);
        unsafe {
            compiler.stack_bound_checks(!opts.no_len_checks);
        }
        let name = module_name();
        compiler.set_module_name(&name);
        Ok(Self { opts, compiler })
    }

    pub(crate) fn jit_compile(
        &mut self,
        bytecode: Bytes,
        spec_id: SpecId,
    ) -> Result<EvmCompilerFn, Error> {
        let name = module_name();
        unsafe {
            self.compiler
                .jit(
                    &name,
                    &bytecode,
                    transmute::<metis_primitives::SpecId, revmc::primitives::SpecId>(spec_id),
                )
                .map_err(|err| Error::Compile(err.to_string()))
        }
    }

    /// Compile in Ahead of Time
    pub(crate) async fn aot_compile(
        &self,
        code_hash: B256,
        bytecode: Bytes,
        spec_id: SpecId,
    ) -> Result<(), Error> {
        let context = Context::create();
        let backend = EvmLlvmBackend::new(&context, true, self.opts.opt_level)
            .map_err(|err| Error::BackendInit(err.to_string()))?;
        let mut compiler = EvmCompiler::new(backend);
        compiler.gas_metering(self.opts.no_gas);
        unsafe {
            compiler.stack_bound_checks(!self.opts.no_len_checks);
        }
        let name = module_name();
        compiler.set_module_name(&name);

        // Compile.
        let _f_id = compiler
            .translate(&name, &bytecode, unsafe {
                transmute::<metis_primitives::SpecId, revmc::primitives::SpecId>(spec_id)
            })
            .map_err(|err| Error::Compile(err.to_string()))?;

        let out_dir = store_path();
        let module_out_dir = out_dir.join(code_hash.to_string());
        fs::create_dir_all(&module_out_dir)?;
        // Write object file
        let obj = module_out_dir.join("a.o");
        compiler
            .write_object_to_file(&obj)
            .map_err(|err| Error::Assembly(err.to_string()))?;
        // Link.
        let so_path = module_out_dir.join("a.so");
        let linker = revmc::Linker::new();
        linker
            .link(&so_path, [obj.to_str().unwrap()])
            .map_err(|err| Error::Link(err.to_string()))?;

        // Delete object files to reduce storage usage
        fs::remove_file(&obj)?;
        Ok(())
    }
}

#[derive(PartialEq, Debug)]
pub enum FetchedFnResult {
    Found(EvmCompilerFn),
    NotFound,
}

pub enum CompileCache {
    JIT(HashMap<B256, EvmCompilerFn>),
    AOT(LruCache<B256, (EvmCompilerFn, Arc<Library>), FxBuildHasher>),
}

impl CompileCache {
    #[inline]
    pub fn is_aot(&self) -> bool {
        matches!(self, CompileCache::AOT(..))
    }

    #[inline]
    pub fn get(&mut self, code_hash: &B256) -> Option<&EvmCompilerFn> {
        match self {
            CompileCache::JIT(jit_cache) => jit_cache.get(code_hash),
            CompileCache::AOT(aot_cache) => aot_cache.get(code_hash).map(|r| &r.0),
        }
    }
}

/// Compiler Worker as an external context.
///
/// External function fetching is optimized by using the memory cache.
/// In many cases, a contract that is called will likely be called again,
/// so the cache helps reduce the library loading cost if we use the AOT mode.
pub struct ExtCompileWorker {
    pool: Option<CompilePool>,
    module_name: String,
    store_path: PathBuf,
}

impl Debug for ExtCompileWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtCompileWorker").finish()
    }
}

impl ExtCompileWorker {
    #[inline]
    pub fn disable() -> Self {
        Self {
            pool: None,
            module_name: module_name(),
            store_path: store_path(),
        }
    }

    #[inline]
    pub fn aot() -> Result<Self, Error> {
        Self::new_with_opts(ExtCompileOptions {
            is_aot: true,
            ..Default::default()
        })
    }

    #[inline]
    pub fn jit() -> Result<Self, Error> {
        Self::new_with_opts(ExtCompileOptions {
            is_aot: false,
            ..Default::default()
        })
    }

    pub fn new_with_opts(opts: ExtCompileOptions) -> Result<Self, Error> {
        let pool = CompilePool::new(
            // Note: the `Context` is not thread safe and cannot be shared across threads.
            // Multiple `Context`s can, however, execute on different threads simultaneously
            // according to the LLVM docs, so we can make it static.
            crate::runtime::get_compiler_context(),
            opts.is_aot,
            opts.threshold,
            opts.pool_size,
            opts.cache_size,
        )?;
        Ok(Self {
            pool: Some(pool),
            module_name: module_name(),
            store_path: store_path(),
        })
    }

    /// Fetches the compiled function from disk, if exists
    pub fn get_function(&self, code_hash: &B256) -> Result<FetchedFnResult, Error> {
        let Some(pool) = &self.pool else {
            return Err(Error::DisableCompiler);
        };
        if code_hash.is_zero() {
            return Ok(FetchedFnResult::NotFound);
        }
        // Write locks are required for reading from LRU Cache
        {
            let mut cache = match pool.cache.try_write() {
                Ok(c) => Some(c),
                Err(err) => match err {
                    /* in this case, read from file instead of cache */
                    TryLockError::WouldBlock => None,
                    TryLockError::Poisoned(err) => Some(err.into_inner()),
                },
            };
            if let Some(cache) = cache.as_deref_mut() {
                // Read lib from the memory cache
                if let Some(t) = cache.get(code_hash) {
                    return Ok(FetchedFnResult::Found(*t));
                }
                // Read lib from the store path if is the AOT mode
                else if let CompileCache::AOT(aot_cache) = cache {
                    let so = self.store_path.join(code_hash.to_string()).join("a.so");
                    if so.try_exists().unwrap_or(false) {
                        {
                            let lib = Arc::new((unsafe { Library::new(so) })?);
                            let f: Symbol<'_, revmc::EvmCompilerFn> =
                                unsafe { lib.get(self.module_name.as_bytes())? };
                            aot_cache.put(*code_hash, (*f, lib.clone()));
                            return Ok(FetchedFnResult::Found(*f));
                        }
                    }
                }
            } else {
                // Use the intepreter to run the code.
                return Err(Error::DisableCompiler);
            }
        }
        Ok(FetchedFnResult::NotFound)
    }

    /// Spawn compile the byecode referred by code_hash
    pub fn spawn(&self, spec_id: SpecId, code_hash: B256, bytecode: Bytes) -> Result<(), Error> {
        if let Some(pool) = &self.pool {
            pool.spawn(spec_id, code_hash, bytecode)?;
        }
        Ok(())
    }

    /// Block compile the byecode referred by code_hash
    pub async fn block(
        &self,
        spec_id: SpecId,
        code_hash: B256,
        bytecode: Bytes,
    ) -> Result<FetchedFnResult, Error> {
        if let Some(pool) = &self.pool {
            let handle = pool.spawn(spec_id, code_hash, bytecode)?;
            let _result = handle.await?;
            self.get_function(&code_hash)
        } else {
            Err(Error::DisableCompiler)
        }
    }
}

#[derive(Debug)]
pub struct ExtCompileOptions {
    pub is_aot: bool,
    pub threshold: u64,
    pub pool_size: usize,
    pub cache_size: usize,
}

impl Default for ExtCompileOptions {
    fn default() -> Self {
        Self {
            is_aot: true,
            threshold: 1,
            pool_size: 3,
            cache_size: 128,
        }
    }
}

/// Compiler handler plugin for the evm
pub struct CompilerHandler<EVM> {
    pub worker: Arc<ExtCompileWorker>,
    _phantom: core::marker::PhantomData<EVM>,
}

impl<EVM> Handler for CompilerHandler<EVM>
where
    EVM: EvmTr<
            Context: ContextTr<Journal: JournalTr<FinalOutput = JournalOutput>>,
            Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
            Instructions: InstructionProvider<
                Context = EVM::Context,
                InterpreterTypes = EthInterpreter,
            >,
        >,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>;
    type Frame = EthFrame<
        EVM,
        EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>,
        <EVM::Instructions as InstructionProvider>::InterpreterTypes,
    >;
    type HaltReason = HaltReason;

    fn frame_call(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
    ) -> Result<FrameInitOrResult<Self::Frame>, Self::Error> {
        let interpreter = &mut frame.interpreter;
        let code_hash = interpreter.bytecode.hash();
        let next_action = match code_hash {
            Some(code_hash) => {
                match self.worker.get_function(&code_hash) {
                    Ok(FetchedFnResult::NotFound) => {
                        // Compile the code
                        let spec_id = evm.ctx().cfg().spec().into();
                        let bytecode = interpreter.bytecode.bytes();
                        let _res = self.worker.spawn(spec_id, code_hash, bytecode);
                        evm.run_interpreter(interpreter)
                    }
                    Ok(FetchedFnResult::Found(_f)) => {
                        // TODO: sync revmc and revm structures for the compiler
                        // https://github.com/paradigmxyz/revmc/issues/75
                        // f.call_with_interpreter_and_memory(interpreter, memory, context)
                        evm.run_interpreter(interpreter)
                    }
                    Err(_) => {
                        // Fallback to the interpreter
                        evm.run_interpreter(interpreter)
                    }
                }
            }
            None => {
                // Fallback to the interpreter
                evm.run_interpreter(interpreter)
            }
        };
        frame.process_next_action(evm, next_action)
    }
}

impl<EVM> Default for CompilerHandler<EVM> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            worker: Arc::new(ExtCompileWorker::disable()),
        }
    }
}

impl<EVM> CompilerHandler<EVM> {
    #[inline]
    pub fn new(worker: Arc<ExtCompileWorker>) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            worker,
        }
    }

    #[inline]
    pub fn aot() -> Result<Self, Error> {
        Ok(Self {
            _phantom: core::marker::PhantomData,
            worker: Arc::new(ExtCompileWorker::aot()?),
        })
    }

    #[inline]
    pub fn jit() -> Result<Self, Error> {
        Ok(Self {
            _phantom: core::marker::PhantomData,
            worker: Arc::new(ExtCompileWorker::jit()?),
        })
    }
}
