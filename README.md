<h1 align="center">Metis SDK: An All-in-One Solution for Building High-performance, Real-time Web3 and AI Ecosystems.</h1>

## Introduction

Metis SDK is redefining the landscape of blockchain scalability, performance, and usability, addressing the pressing demands of the rapidly evolving Web3 ecosystem. As decentralized applications (DApps) continue to gain traction, the need for infrastructure capable of handling massive transaction volumes, lightning-fast processing speeds, and frictionless user experiences has never been greater. Metis rises to this challenge by introducing the SDK solution to deliver unparalleled throughput, ultra-low latency, and seamless transaction finality.

## Features

- **Modular Architecture**: Metis SDK is an open-source modular framework designed to provide Web3 developers with a powerful foundation for innovation. It provides plugin features, you can choose according to your needs.
- **High Performance**: Leverages [MetisVM](crates/vm/README.md), [MetisDB](crates/db/README.md) and [parallel execution](crates/pe/README.md) technologies to deliver low-latency, high-throughput on-chain operations.
- **Strong Compatibility**: Seamlessly integrates with the Ethereum ecosystem, supporting Solidity smart contracts while simplifying DApp migration.
- **Scalable Consensus Engine**: Supports customizable Sequencer networks, which can be either decentralized or centralized, depending on the specific use case.
- **Decentralized Sequencer**: For Layer 2, the Metis Sequencer Network consists of multiple decentralized sequencers, each responsible for transaction ordering, validation, and state commitment.
- **Flexible Deployment**: Supports Layer 1, Layer 2, and Layer 3 network configurations, tailored to diverse application needs.
- **AI Features Support**: Use [LazAI](https://lazai.network) and [Alith](https://github.com/0xLazAI/alith) to provide AI related on-chain or off-chain tasks including data processing, model inference, fine tuning, etc.

## Contributing

We welcome any contributions to Metis SDK! See the [contributing guide](./docs/CONTRIBUTING.md) for more information.

## Acknowledgements

We ❤️ open-source! Metis SDK stands on the shoulders of these giants in the blockchain ecosystem:

- [Alloy](https://github.com/alloy-rs) - Foundational Rust libraries for Ethereum and other EVM-based chains.
- [Reth](https://github.com/paradigmxyz/reth) - Modular, contributor-friendly and blazing-fast implementation of the Ethereum protocol in Rust.
- [Revm](https://github.com/bluealloy/revm/) - Rust implementation of the EVM.
- [Revmc](https://github.com/paradigmxyz/revmc) - JIT and AOT compiler for the EVM.
- [Pevm](https://github.com/risechain/pevm) - Extremely fast parallel EVM used in the RiseChain.

## License

Apache 2.0
