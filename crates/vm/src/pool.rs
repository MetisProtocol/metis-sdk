use crate::{compiler::CompileCache, error::Error};
use lru::LruCache;
use metis_primitives::B256;
use rustc_hash::FxBuildHasher;

use super::{
    compiler::{CompileOptions, Compiler, CompilerContext},
    runtime::get_runtime,
};
use metis_primitives::{Bytes, SpecId};
use std::{
    num::NonZeroUsize,
    ops::DerefMut,
    sync::{Arc, RwLock},
};
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinHandle,
};

/// A compile pool for compiling bytecode to the native code with the JIT or AOT
/// compile cache.
pub struct CompilePool {
    pub threshold: u64,
    pub cache: Arc<RwLock<CompileCache>>,
    semaphore: Arc<Semaphore>,
    inner: Arc<PoolInner>,
}

struct PoolInner {
    compiler: Mutex<Compiler<'static>>,
}

impl CompilePool {
    /// Creates a new `CompilePool`.
    ///
    /// # Arguments
    ///
    /// * `is_aot` - Use the AOT compiler or the JIT compiler
    /// * `threshold` - The threshold for the number of times a bytecode must be seen before it is
    ///   compiled.
    /// * `max_concurrent_tasks` - The maximum number of concurrent aot compiling tasks allowed.
    /// * `cache_size` - RLU cache size.
    #[inline]
    pub(crate) fn new(
        context: &'static CompilerContext,
        is_aot: bool,
        threshold: u64,
        max_concurrent_tasks: usize,
        cache_size: usize,
    ) -> Result<Self, Error> {
        Self::new_with_config(
            context,
            threshold,
            max_concurrent_tasks,
            cache_size,
            CompileOptions {
                is_aot,
                ..Default::default()
            },
        )
    }

    pub(crate) fn new_with_config(
        context: &'static CompilerContext,
        threshold: u64,
        max_concurrent_tasks: usize,
        cache_size: usize,
        opts: CompileOptions,
    ) -> Result<Self, Error> {
        let is_aot = opts.is_aot;
        Ok(Self {
            threshold,
            semaphore: Arc::new(Semaphore::new(max_concurrent_tasks)),
            inner: Arc::new(PoolInner {
                compiler: Mutex::new(Compiler::new(context, opts)?),
            }),
            cache: Arc::new(RwLock::new(if is_aot {
                CompileCache::AOT(LruCache::with_hasher(
                    NonZeroUsize::new(cache_size).ok_or(Error::Internal("".to_string()))?,
                    FxBuildHasher,
                ))
            } else {
                CompileCache::JIT(Default::default())
            })),
        })
    }

    /// Spawns a compilation task for the given bytecode with the specified specId.
    ///
    /// # Arguments
    ///
    /// * `spec_id` - The specification ID for the EVM.
    /// * `code_hash` - The hash of the bytecode to be compiled.
    /// * `bytecode` - The bytecode to be compiled.
    ///
    /// # Returns
    ///
    /// A `JoinHandle` to the spawned task, which resolves to a `Result` indicating success or
    /// failure.
    pub(crate) fn spawn(
        &self,
        spec_id: SpecId,
        code_hash: B256,
        bytecode: Bytes,
    ) -> Result<JoinHandle<Result<(), Error>>, Error> {
        let semaphore = self.semaphore.clone();
        let inner = self.inner.clone();
        let cache = self.cache.clone();
        let runtime = get_runtime();
        let handle = runtime.spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            // Check if the bytecode is all zeros
            if code_hash.is_zero() {
                return Ok(());
            }
            let mut compiler = inner.compiler.lock().await;
            if compiler.opts.is_aot {
                compiler.aot_compile(code_hash, bytecode, spec_id).await?;
            } else {
                let jit = compiler.jit_compile(bytecode, spec_id)?;
                let mut cache = cache
                    .write()
                    .map_err(|err| Error::LockPoison(err.to_string()))?;
                if let CompileCache::JIT(jit_cache) = cache.deref_mut() {
                    jit_cache.insert(code_hash, jit);
                }
            }
            Ok(())
        });
        Ok(handle)
    }
}
