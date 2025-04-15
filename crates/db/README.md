# MetisDB: High-Performance State Management for Scalable Blockchain Systems

MetisDB is a cutting-edge storage solution tailored for high-throughput, low-latency Layer 2 networks. Designed to optimize transaction processing, state synchronization, and historical data management, MetisDB separates state commitment from state storage, ensuring unmatched performance and scalability.

## Core Design Principles

- **State Commitment and Storage Separation**
    - The State Commitment Layer manages active blockchain state, ensuring fast access and updates.
    - The State Storage Layer focuses on historical data, optimized for archival and RPC nodes.
- **Flexible Merkle Tree Support**
    - Supports multiple Merkle tree types (e.g., Sparse Merkle Trees, IAVL Trees) to meet diverse blockchain needs.
- **Multi-Version Concurrency Control (MVCC)**
    - Enables concurrent state access and efficient rollbacks in parallel execution environments.
- **Asynchronous I/O**
    - Reduces latency by asynchronously committing state updates, ensuring high throughput without delays.

## Key Components

### State Commitment Layer

Handles the active blockchain state for fast transaction validation and execution:

- **Memory-Mapped Merkle Tree**: Provides nanosecond-level state access for high-speed operations.
- **Transaction Delta Processing**: Applies state changes efficiently, generating block hashes from updated Merkle roots.
- **Write-Ahead Log (WAL)**: Ensures durability by recording changesets asynchronously, enabling quick recovery.
Snapshot Management: Periodic snapshots minimize WAL size and accelerate recovery times.

### State Storage Layer

Optimized for historical data storage and retrieval:

- **Raw Key-Value Storage**: Reduces overhead with minimal metadata, leveraging LSM tree backends like RocksDB.
- **Asynchronous Pruning**: Removes outdated data without hindering active processes.
- **Flexible Backends**: Supports various database solutions to cater to infrastructure requirements.

### Execution Workflow

- **Transaction Submission**: Transactions are forwarded by the Sequencer to the Parallel Execution Framework.
- **State Access and Update**: The State Commitment Layer retrieves and updates the active state using memory-mapped Merkle trees.
- **Commit Process**: Updated Merkle roots are committed, with changesets logged asynchronously to the State Storage Layer.
- **Historical Queries**: Archival nodes efficiently serve historical state queries using the State Storage Layer.

### Key Innovations

- **Decoupled State Layers**
    - Faster state access for active processes.
    - Reduced overhead for historical queries.
- **MVCC**
    - Enhances concurrent state access and rollback efficiency.
- **Optimized Asynchronous I/O**
    - Allows continuous transaction processing by deferring disk writes.

### Performance Highlights

- **State Synchronization**
    - Up to 10x faster sync times with optimized WAL replay and memory-mapped access.
- **Transaction Throughput**
    - Handles tens of thousands of transactions per second with parallel execution and efficient I/O.
- **Storage Efficiency**
    - Reduces storage size by up to 50% through pruning and schema optimizations.
