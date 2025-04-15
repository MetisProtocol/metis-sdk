# 🚀 Parallel EVM (Block-STM Based)

This crate implements **parallel execution of Ethereum Virtual Machine (EVM) transactions** using a **Block-STM** mechanism. It guarantees that, under a given transaction order, the final state and outputs are **deterministically consistent** with serial execution. Designed for high throughput and efficiency, this system is built upon the foundation of [PEVM](https://github.com/risechain/pevm) with critical enhancements in scheduling, memory management, and conflict resolution. The key improvements are:

1. A more efficient scheduling algorithm to minimize transaction re-execution and re-verification;

2. Optimized data structures utilizing lock-free mechanisms to reduce synchronization overhead;

3. An enhanced transaction prior-knowledge system to infer dependencies and reduce execution conflicts via parallel-aware scheduling.

---

## ✨ Key Features

- 🔄 **Deterministic Parallel Execution**: Ensures outputs are consistent with serial execution under a fixed order.
- ⚡ **Optimized Scheduler**: Efficiently schedules transactions to minimize re-execution and re-validation.
- 🧠 **Pre-execution Analysis**: Utilizes transaction dependency graphs for better conflict avoidance.
- 🔒 **Lock-Free Data Structures**: Improves throughput by reducing lock contention.
- 🧬 **Multi-Versioned Memory (MVMemory)**: Provides STM-style isolation with per-transaction versioning of state.
- 🔧 **RETH Integration**: Leverages the high-performance [reth](https://github.com/paradigmxyz/reth) EVM as a backend execution engine.

---

## 🧱 Architecture Overview

### 📦 Modules

#### `DB`
- Implements an in-memory database (Memory DB) to simulate the blockchain state during transaction execution.

#### `Executor`
- Entry point for executing a block of transactions.
- Supports **serial** and **parallel** execution modes.
- Delegates transaction scheduling, execution, validation, and finalization.

#### `Scheduler`
- Coordinates the execution and re-execution of transactions.
- Integrates a `TransactionGraph` to encode known transaction dependencies and derive an optimized scheduling order.
- The Execution Flow is:
  1. **Optimistically executes** all transactions in parallel;
  2. After execution, verifying that the **read set** was not modified by prior transactions — if so, re-execute;
  3. Re-execute all transactions whose read sets have been modified by an executed transaction.
  4. Finalizes once all transactions are successfully executed and verified.

#### `MVMemory`
- Maintains multiple versions of key-value pairs written by transactions.
- Ensures a transaction reads the **latest version committed by a prior transaction**.
- Tracks each transaction's **read and write sets** for conflict detection.

#### `vm`
- Wraps `revm`, the Rust-based Ethereum Virtual Machine implementation.
- Provides abstraction for executing EVM transactions in a concurrency-aware manner.
- `VmDB` serves as a multi-version memory interface compatible with `revm`.

---

## 📊 Benchmarking

Benchmarking is performed using [Criterion](https://crates.io/crates/criterion). To run the benchmarks:

```bash
JEMALLOC_SYS_WITH_MALLOC_CONF="thp:always,metadata_thp:always" cargo bench --bench gigagas
```

## 🙏 Acknowledgments
This crate builds upon the work of:

[PEVM](https://github.com/risechain/pevm) – A Block-STM based parallel EVM.

[RETH](https://github.com/paradigmxyz/reth) – A high-performance Rust implementation of the EVM, used as the core execution backend in this crate.

Special thanks to the authors and contributors of these foundational libraries.

