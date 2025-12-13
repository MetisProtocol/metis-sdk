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
