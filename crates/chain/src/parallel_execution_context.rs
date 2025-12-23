//! Parallel Execution Context - Execution state management
//!
//! This module tracks the execution context and ensures post_execution
//! is called exactly once per block execution.

use alloy_primitives::TxHash;
use std::collections::HashMap;

/// Execution mode for parallel block executor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Payload Builder mode - constructing new blocks (parallel execution)
    Builder,
    /// Payload Validator mode - validating received blocks (serial execution)
    Validator,
    /// Block Import mode - importing/syncing blocks (parallel execution)
    Import,
}

/// Cached result from parallel execution
#[derive(Debug, Clone)]
pub struct CachedExecutionResult {
    pub gas_used: u64,
    pub receipt: reth_primitives::Receipt,
}

/// Context for tracking parallel execution state
#[derive(Debug)]
pub struct ParallelExecutionContext {
    /// Whether post_execution has been called for this block
    pub post_execution_called: bool,
    /// Current execution mode
    pub execution_mode: ExecutionMode,
    /// Cache of parallel execution results (tx_hash -> result)
    /// Used in payload builder to avoid re-executing transactions
    pub execution_cache: HashMap<TxHash, CachedExecutionResult>,
}

impl ParallelExecutionContext {
    /// Create a new execution context
    pub fn new(mode: ExecutionMode) -> Self {
        Self {
            post_execution_called: false,
            execution_mode: mode,
            execution_cache: HashMap::new(),
        }
    }

    /// Check if parallel execution should be used based on mode
    pub fn should_execute_in_parallel(&self) -> bool {
        matches!(
            self.execution_mode,
            ExecutionMode::Builder | ExecutionMode::Import
        )
    }

    /// Reset the context for a new block
    pub fn reset(&mut self) {
        self.post_execution_called = false;
        self.execution_cache.clear();
    }

    /// Mark post_execution as called
    pub fn mark_post_execution_called(&mut self) {
        self.post_execution_called = true;
    }

    /// Cache execution result for a transaction
    pub fn cache_result(
        &mut self,
        tx_hash: TxHash,
        gas_used: u64,
        receipt: reth_primitives::Receipt,
    ) {
        self.execution_cache
            .insert(tx_hash, CachedExecutionResult { gas_used, receipt });
    }

    /// Get cached execution result
    pub fn get_cached_result(&self, tx_hash: &TxHash) -> Option<&CachedExecutionResult> {
        self.execution_cache.get(tx_hash)
    }

    /// Check if we have cached results (indicating parallel execution was done)
    pub fn has_cached_results(&self) -> bool {
        !self.execution_cache.is_empty()
    }
}

impl Default for ParallelExecutionContext {
    fn default() -> Self {
        // Default to Builder mode (parallel execution)
        Self::new(ExecutionMode::Builder)
    }
}

// Thread-local global cache for parallel execution results
// This is used by payload builder to pass results to the executor
use std::cell::RefCell;

thread_local! {
    static GLOBAL_EXECUTION_CACHE: RefCell<HashMap<TxHash, CachedExecutionResult>> = RefCell::new(HashMap::new());
}

/// Set global cache with parallel execution results
/// Called by payload builder before serial execution loop
pub fn set_global_cache(tx_hashes: &[TxHash], results: &[metis_pe::TxExecutionResult]) {
    GLOBAL_EXECUTION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.clear();
        for (tx_hash, result) in tx_hashes.iter().zip(results.iter()) {
            cache.insert(
                *tx_hash,
                CachedExecutionResult {
                    gas_used: result.receipt.cumulative_gas_used,
                    receipt: result.receipt.clone(),
                },
            );
        }
        tracing::info!(target: "metis::parallel",
            "💾 Set global cache with {} parallel execution results",
            cache.len()
        );
    });
}

/// Get cached result from global cache
/// Called by executor during execute_transaction
pub fn get_global_cached_result(tx_hash: &TxHash) -> Option<CachedExecutionResult> {
    GLOBAL_EXECUTION_CACHE.with(|cache| {
        let result = cache.borrow().get(tx_hash).cloned();
        if result.is_some() {
            tracing::debug!(target: "metis::parallel",
                "🎯 Cache HIT for tx 0x{:x}", tx_hash);
        } else {
            tracing::debug!(target: "metis::parallel",
                "❌ Cache MISS for tx 0x{:x}", tx_hash);
        }
        result
    })
}

/// Clear global cache
/// Called after block execution completes
pub fn clear_global_cache() {
    GLOBAL_EXECUTION_CACHE.with(|cache| {
        let cleared_count = cache.borrow().len();
        cache.borrow_mut().clear();
        tracing::debug!(target: "metis::parallel",
            "🧹 Cleared global cache ({} entries removed)",
            cleared_count
        );
    });
}

/// Get cache statistics (for debugging)
pub fn get_cache_stats() -> (usize, usize) {
    GLOBAL_EXECUTION_CACHE.with(|cache| {
        let len = cache.borrow().len();
        let capacity = cache.borrow().capacity();
        (len, capacity)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_modes() {
        let builder_ctx = ParallelExecutionContext::new(ExecutionMode::Builder);
        assert!(builder_ctx.should_execute_in_parallel());
        assert!(!builder_ctx.post_execution_called);

        let validator_ctx = ParallelExecutionContext::new(ExecutionMode::Validator);
        assert!(!validator_ctx.should_execute_in_parallel());

        let import_ctx = ParallelExecutionContext::new(ExecutionMode::Import);
        assert!(import_ctx.should_execute_in_parallel());
    }

    #[test]
    fn test_context_reset() {
        let mut ctx = ParallelExecutionContext::default();
        ctx.mark_post_execution_called();
        assert!(ctx.post_execution_called);

        ctx.reset();
        assert!(!ctx.post_execution_called);
    }
}
