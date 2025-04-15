# MetisVM: An AI-Native and High-Performance Virtual Machine for Smart Contracts

## Introduction

MetisVM is compatible with Ethereum and provides corresponding infrastructure support for AI applications. It offers unparalleled efficiency and performance for the execution of smart contracts such as in DeFi, on-chain gaming, and AI inference. Optimizing for both traditional smart contracts and AI-specific operations enables MetisVM to create the foundation that powers the AI and Web3 ecosystem.

## Technical Breakthroughs and Innovation Achievements

### Dynamic Opcode Optimization

MetisVM achieves exceptional performance through two key optimizations: advanced opcode processing and custom instruction extensions. These innovations, described below, allow for faster execution with lower costs while maintaining full compatibility with existing smart contracts.

#### JIT/AOT Compilation Optimization

MetisVM employs a sophisticated dynamic opcode optimization mechanism that analyzes the bytecode of smart contracts in real time. It can identify frequently-used opcodes and apply Just In Time (JIT) and Ahead Of Time (AOT) compilation techniques to reduce execution costs. This not only enhances contract efficiency but also minimizes users' gas consumption. Besides, MetisVM saves time on repeated compilation through compilation pooling and caching technology.

#### Instruction and Precompile Extension

MetisVM development environment, while being compatible with the EVM, supports customizable extended opcodes (such as floating point numbers of different precisions to support AI quantization models with different precision requirements for inference through WASM).

### Speculative & Parallel Execution

The speculative and parallel execution of MetisVM utilizes advanced predictive algorithms including static code analysis to forecast the results of certain contract operations. Based on these predictions, it can execute multiple transactions in parallel, thereby significantly increasing transaction throughput. The optimized resource allocation ensures that the system can handle a large number of concurrent transactions without compromising security or accuracy.

### State-Aware Caching

MetisVM State-aware caching intelligently caches the frequently-accessed state variables in smart contracts. By tracking the state transitions of contracts, it reduces the need for redundant storage access. This greatly improves the execution speed, especially for state-intensive contracts such as those used for governance and voting in Decentralized Autonomous Organizations (DAOs). The caching mechanism is designed to be efficient and adaptive, ensuring that the cached data is always up-to-date and accurate.

### AI Infrastructure Support

MetisVM provides foundational support for on-chain AI applications through three critical innovations. By optimizing inference engines, leveraging hardware acceleration, and incorporating TEE and zero-knowledge proofs, MetisVM establishes an environment where AI models can operate efficiently and securely within blockchain infrastructure.

#### Inference Engine Optimization

MetisVM optimizes and supports compute-intensive on-chain inference AI applications through a combination of inference engine optimization, VM precompilation, and host functions. The inference engine optimization ensures that AI models can run efficiently on the blockchain, thereby reducing the latency of AI-based smart contract operations. VM precompilation allows for faster loading and execution of AI models, while host functions provide a seamless interface between AI models and smart contract logic.

#### AI Coprocessor Acceleration

Machine learning inference often requires a large amount of computing resources, and MetisVM can utilize various hardware accelerators such as SIMD (e.g., AVX512), GPU, TPU, and FPGA. Through hardware acceleration, the inference performance can be significantly enhanced, thereby speeding up the operation of applications and accomplishing functions like on-chain model inference.

### Developer Ecosystem Construction

MetisVM will deliver a comprehensive toolkit designed to streamline AI integration for blockchain developers. Combining familiar EVM development tools with specialized AI capabilities, creates an increased accessibility for builders where both traditional smart contract developers and AI engineers can build sophisticated applications without sacrificing performance or compatibility.

The ecosystem includes industry-standard model interfaces, multi-language support, and seamless integration with popular machine learning frameworks. This hybrid approach enables developers to deploy complex AI applications - from real-time financial derivatives and onchain games to intelligent risk management systems - all while maintaining the security and transparency of blockchain infrastructure.

#### EVM Compatible Toolkit

Developers can use EVM compatible tool chains such as Foundry, Hardhat, etc. to complete contract development, testing, debugging and deployment.

#### AI Contract Template Library

Pre-configured with over 20 industry-standard model interfaces (such as the MetisVM-adapted versions of GPT, Llama, DeepSeek and Stable Diffusion models).

#### Hybrid Development Environment

Supports mixed-programming in multiple languages including Solidity, Rust, and Python. It offers a one-click packaging tool process for AI models to smart contracts. Developers can use mainstream machine learning frameworks like Pytorch, Tensorflow, and ONNX to train and utilize models without the need for additional modifications or conversions to the models, greatly simplifying the development process.

## Developing

See the [developing guide](./docs/dev.md) for more information.

## License

Apache 2.0
