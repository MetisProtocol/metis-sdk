//! Test small blocks that we have specific handling for, like implicit fine-tuning
//! the concurrency level, falling back to sequential processing, etc.

use metis_pe::InMemoryDB;

pub mod common;

#[test]
fn empty_revm_block() {
    common::test_execute_revm(InMemoryDB::default(), Vec::new());
}
