use crate::{
    env::{module_name, store_path},
    error::Error,
    hotcode::HotCodeCounter,
    pool::CompilePool,
};
use libloading::{Library, Symbol};
use lru::LruCache;
use metis_primitives::{B256, Bytes, SpecId};
use revm::{Database, handler::register::EvmHandler};
use revmc::EvmCompilerFn;
use revmc::{EvmCompiler, EvmLlvmBackend, OptimizationLevel, llvm::Context};
use rustc_hash::FxBuildHasher;
use rustc_hash::FxHashMap as HashMap;
use std::fs;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, TryLockError},
};

pub use revmc::llvm::Context as CompilerContext;

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
    pub(crate) fn new(context: &'ctx Context, opts: CompileOptions) -> Result<Self, Error> {
        let backend = EvmLlvmBackend::new(context, opts.is_aot, OptimizationLevel::Aggressive)
            .map_err(|err| Error::BackendInit(err.to_string()))?;
        let mut compiler = EvmCompiler::new(backend);
        compiler.gas_metering(opts.no_gas);
        unsafe {
            compiler.stack_bound_checks(opts.no_len_checks);
        }
        let name = module_name();
        compiler.set_module_name(&name);
        compiler.inspect_stack_length(true);
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
                .jit(&name, &bytecode, spec_id)
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
            compiler.stack_bound_checks(self.opts.no_len_checks);
        }
        let name = module_name();
        compiler.set_module_name(&name);
        compiler.inspect_stack_length(true);

        // Compile.
        let _f_id = compiler
            .translate(&name, &bytecode, spec_id)
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
    pool: CompilePool,
}

impl ExtCompileWorker {
    #[inline]
    pub fn new_aot(context: &CompilerContext) -> Result<Self, Error> {
        Self::new_with_opts(
            context,
            ExtCompileOptions {
                is_aot: true,
                ..Default::default()
            },
        )
    }

    #[inline]
    pub fn new_jit(context: &CompilerContext) -> Result<Self, Error> {
        Self::new_with_opts(
            context,
            ExtCompileOptions {
                is_aot: false,
                ..Default::default()
            },
        )
    }

    pub fn new_with_opts(
        context: &CompilerContext,
        opts: ExtCompileOptions,
    ) -> Result<Self, Error> {
        let hot_code_counter = HotCodeCounter::new(opts.primary, opts.pool_size)?;
        let pool = CompilePool::new(
            // Note: the `Context` is not thread safe and cannot be shared across threads.
            // Multiple `Context`s can, however, execute on different threads simultaneously
            // according to the LLVM docs, so we can make it static.
            unsafe { std::mem::transmute::<&CompilerContext, &CompilerContext>(context) },
            opts.is_aot,
            opts.threshold,
            hot_code_counter,
            opts.pool_size,
            opts.cache_size,
        )?;
        Ok(Self { pool })
    }

    /// Fetches the compiled function from disk, if exists
    pub fn get_function(&self, code_hash: &B256) -> Result<FetchedFnResult, Error> {
        if code_hash.is_zero() {
            return Ok(FetchedFnResult::NotFound);
        }
        // Write locks are required for reading from LRU Cache
        {
            let mut cache = match self.pool.cache.try_write() {
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
                    let name = module_name();
                    let so = store_path().join(code_hash.to_string()).join("a.so");
                    if so.try_exists().unwrap_or(false) {
                        {
                            let lib = Arc::new((unsafe { Library::new(so) })?);
                            let f: Symbol<'_, revmc::EvmCompilerFn> =
                                unsafe { lib.get(name.as_bytes())? };
                            aot_cache.put(*code_hash, (*f, lib.clone()));
                            return Ok(FetchedFnResult::Found(*f));
                        }
                    }
                }
            }
        }
        Ok(FetchedFnResult::NotFound)
    }

    /// Spwan compile the byecode referred by code_hash
    pub fn spwan(&self, spec_id: SpecId, code_hash: B256, bytecode: Bytes) -> Result<(), Error> {
        self.pool.spwan(spec_id, code_hash, bytecode)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct ExtCompileOptions {
    pub is_aot: bool,
    pub primary: bool,
    pub threshold: u64,
    pub pool_size: usize,
    pub cache_size: usize,
}

impl Default for ExtCompileOptions {
    fn default() -> Self {
        Self {
            is_aot: true,
            primary: true,
            threshold: 1,
            pool_size: 3,
            cache_size: 128,
        }
    }
}

/// Register handler for external context to support background compile worker in node runtime
pub fn register_compile_handler<DB: Database + 'static>(
    handler: &mut EvmHandler<'_, Arc<ExtCompileWorker>, DB>,
) {
    let prev = handler.execution.execute_frame.clone();
    handler.execution.execute_frame = Arc::new(move |frame, memory, tables, context| {
        let interpreter = frame.interpreter_mut();
        let code_hash = interpreter.contract.hash.unwrap_or_default();

        match context.external.get_function(&code_hash) {
            Ok(FetchedFnResult::NotFound) => {
                // Compile the code
                let spec_id = context.evm.inner.spec_id();
                let bytecode = context.evm.db.code_by_hash(code_hash).unwrap_or_default();
                let _res = context
                    .external
                    .spwan(spec_id, code_hash, bytecode.original_bytes());
                prev(frame, memory, tables, context)
            }
            Ok(FetchedFnResult::Found(f)) => {
                let res = catch_unwind(AssertUnwindSafe(|| unsafe {
                    f.call_with_interpreter_and_memory(interpreter, memory, context)
                }));
                Ok(res.unwrap())
            }
            Err(_) => {
                // Fallback to the interpreter
                prev(frame, memory, tables, context)
            }
        }
    });
}
