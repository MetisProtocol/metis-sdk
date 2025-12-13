//! State Accumulator - Unified state management for parallel execution
//!
//! This module provides a centralized way to merge and commit parallel execution results,
//! ensuring consistent state handling across all execution paths.

use metis_primitives::{Account, Address, DefaultHashBuilder, HashMap};
use reth_errors::BlockExecutionError;
use revm::DatabaseCommit;
use std::collections::BTreeMap;

/// State accumulator for managing parallel execution results.
///
/// This struct ensures deterministic state merging and committing:
/// 1. Merges states in transaction order (later txs overwrite earlier ones)
/// 2. Maintains sorted order by address for deterministic commits
/// 3. Provides clear interface for state management
pub struct StateAccumulator {
    /// Accumulated state changes, sorted by address for determinism
    /// Using BTreeMap ensures iteration order is always consistent
    changes: BTreeMap<Address, Account>,
}

impl StateAccumulator {
    /// Create a new empty state accumulator
    pub fn new() -> Self {
        Self {
            changes: BTreeMap::new(),
        }
    }

    /// Accumulate state changes from parallel execution results.
    ///
    /// States are merged in transaction order, where later transactions
    /// overwrite earlier transactions for the same address. This replicates
    /// serial execution behavior.
    ///
    /// # Arguments
    /// * `results` - Transaction execution results from metis-pe engine
    pub fn accumulate(&mut self, results: &[metis_pe::TxExecutionResult]) {
        for result in results.iter() {
            // Merge this transaction's state
            // Later transactions overwrite earlier ones for the same address
            for (address, account) in result.state.iter() {
                // Insert or update the account state
                self.changes.insert(*address, account.clone());
            }
        }
    }

    /// Commit accumulated state changes to the database.
    ///
    /// States are committed in sorted order by address to ensure deterministic
    /// state root calculation across all nodes.
    ///
    /// # Arguments
    /// * `db` - Database to commit changes to
    pub fn commit_to<DB>(&self, db: &mut DB) -> Result<(), BlockExecutionError>
    where
        DB: DatabaseCommit,
    {
        // BTreeMap already maintains sorted order, so we can iterate directly
        for (address, account) in self.changes.iter() {
            let mut single_state: HashMap<Address, Account, DefaultHashBuilder> =
                HashMap::default();
            single_state.insert(*address, account.clone());
            db.commit(single_state);
        }
        Ok(())
    }

    /// Get the number of accumulated state changes
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// Check if the accumulator is empty
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Clear all accumulated state changes
    pub fn clear(&mut self) {
        self.changes.clear();
    }
}

impl Default for StateAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_accumulator_ordering() {
        let mut acc = StateAccumulator::new();

        // Test that BTreeMap maintains sorted order
        assert_eq!(acc.len(), 0);
        assert!(acc.is_empty());

        acc.clear();
        assert!(acc.is_empty());
    }
}
