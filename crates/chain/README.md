# Chain

This document provides a guide on utilizing the Metis SDK to rapidly construct blockchain nodes. The implementation leverages parallel transaction execution to enhance performance while maintaining compatibility with Ethereum's execution layer standards.

## Quick Start

### Build from Source

Firstly, see [here](../../CONTRIBUTING.md) to install build dependencies. Then install the `metis` command in the root of the repo.

```shell
make install
```

Or build with the max performance mode

```shell
make PROFILE=maxperf install
```

Execute the following command to launch a Metis devnet:

```shell
metis node --dev --dev.block-time 2s --http --ws
```

Execute the following command to launch a Metis devnet with the genesis config file:

```shell
metis node --dev --dev.block-time 2s --http --ws --chain genesis.json
```

A `genesis.json` example is

```json
{
    "nonce": "0x42",
    "timestamp": "0x0",
    "extraData": "0x5343",
    "gasLimit": "0x989680",
    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "alloc": {
        "0xf47559c312c063ab186fa9cbada4a8d3411e7bef": {
            "balance": "0x4a47e3c12448f4ad000000"
        },
        "0x9c82df3861e1c1e9bb3a5c5eb0a8dd2f69e755a7": {
            "balance": "0x4a0"
        }
    },
    "number": "0x0",
    "gasUsed": "0x0",
    "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "difficulty": "0x1",
    "coinbase": "0xf47559c312c063ab186fa9cbada4a8d3411e7bef",
    "config": {
        "ethash": {
            "fixedDifficulty": 1,
            "minimumDifficulty": 1
        },
        "chainId": 133717,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "terminalTotalDifficulty": 0,
        "terminalTotalDifficultyPassed": true,
        "shanghaiTime": 0
    }
}
```

> Note that these commands will not open any HTTP/WS ports by default. You can change this by adding the --http, --ws flags, respectively and using the --http.api and --ws.api flags to enable various ETH compatible JSON-RPC APIs.

### Docker

Metis engine node docker images for both x86_64 and ARM64 machines are published with every release of reth on GitHub Container Registry.

```shell
docker pull ghcr.io/metisprotocol/metis
```

Or a specific version (e.g. v0.1.0) with:

```shell
docker pull ghcr.io/metisprotocol/metis:v0.1.0
```

You can test the image with:

```shell
docker run --rm ghcr.io/metisprotocol/metis --version
```

To run the dev node with Docker, run:

```shell
docker run \
    -v data:/root/.local/share/reth/ \
    -d \
    -p 8545 \
    -p 8546 \
    -p 9001:9001 \
    -p 30303:30303 \
    -p 30303:30303/udp \
    --name metis \
    ghcr.io/metisprotocol/metis \
    node \
    --datadir=data \
    --metrics 0.0.0.0:9001 \
    --dev \
    --dev.block-time 2s --chain genesis.json \
    --http --http.api=all --http.addr=0.0.0.0 --http.port=8545 \
    --ws --ws.api=all --ws.addr=0.0.0.0 --ws.port=8546
```

### Observability with Prometheus & Grafana

Metis chain exposes a number of metrics which can be enabled by adding the `--metrics` flag:

```shell
metis node --dev --dev.block-time 2s --http --ws --chain genesis.json --metrics 127.0.0.1:9001
```

When you use metis chain for deployment, you can expose this ports

```text
# Expose the HTTP RPC Port 8545.
8545
# Expose the WS RPC Port 8546.
8546
# Expose the 9001 for metric.
9001
# Expose the 30303 port (TCP and UDP) for peering with other nodes.
30303
30303/udp
```

## CLI Reference

Metis chain is built on reth, they use almost the same CLI parameters, you can find more CLI information [here](https://reth.rs/cli/reth).

## Interacting with Metis Chain over JSON-RPC

Metis chain is built on reth, they both have the same ETH compatible JSON-RPC, you can find more JSON-RPC information [here](https://reth.rs/jsonrpc/intro).

## Core Components

### `ParallelExecutorBuilder`

The `ParallelExecutorBuilder` implements the `ExecutorBuilder` trait, serving as a factory for creating parallelized EVM executors.

```rust
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct ParallelExecutorBuilder;

impl ExecutorBuilder for ParallelExecutorBuilder {
    async fn build_evm(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<(Self::EVM, Self::Executor)> {
        // Implementation details
    }
}
```

### `ParallelExecutor`

During specific execution, the parallel executor needs to process 3 stages of block content:

- Applies any necessary changes before executing the block's transactions.
- Executing block transactions
- Applies any necessary changes after executing the block's transactions, completes execution and returns the underlying EVM along with execution result.

```rust
pub struct ParallelExecutor<DB> {
    strategy_factory: EthEvmConfig,
    db: State<DB>,
    executor: metis_pe::ParallelExecutor,
    concurrency_level: NonZeroUsize,
}

// perform 3 steps to complete the block calculation
fn execute_one(
    &mut self,
    block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {}

// parallel execute block transactions which belongs to the second stage
pub fn execute_block(
    &mut self,
    block: &RecoveredBlock<<<Self as Executor<DB>>::Primitives as NodePrimitives>::Block>,
) -> Result<BlockExecutionResult<Receipt>, BlockExecutionError> {}
```

### NodeBuilder

We use general `reth::builder::NodeBuilder<ChainSpec>` to build chain nodes. `NodeBuilder` provides execution plugins. The advantage of this is that the metis-sdk provider can be run in a specific node as a plugin.

```rust
pub struct NodeBuilder<DB, ChainSpec> {
    /// All settings for how the node should be configured.
    config: NodeConfig<ChainSpec>,
    /// The configured database for the node.
    database: DB,
}
```

The `ChainSpec` comes from genesis file, for example:

```rust
pub fn custom_chain() -> Arc<ChainSpec> {
    let custom_genesis = r#"
{
    "nonce": "0x42",
    "timestamp": "0x0",
    "extraData": "0x5343",
    "gasLimit": "0x5208",
    "difficulty": "0x400000000",
    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "coinbase": "0x0000000000000000000000000000000000000000",
    "alloc": {
        "0x6Be02d1d3665660d22FF9624b7BE0551ee1Ac91b": {
            "balance": "0x4a47e3c12448f4ad000000"
        }
    },
    "number": "0x0",
    "gasUsed": "0x0",
    "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "config": {
        "ethash": {},
        "chainId": 2600,
        "homesteadBlock": 0,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "terminalTotalDifficulty": 0,
        "terminalTotalDifficultyPassed": true,
        "shanghaiTime": 0
    }
}
"#;
    let genesis: Genesis = serde_json::from_str(custom_genesis).unwrap();
    Arc::new(genesis.into())
}
```

We can also use `reth_ethereum::chainspec::ChainSpec` dependency, which has the general genesis configuration of various chains:

```rust
pub static MAINNET: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {..});
pub static SEPOLIA: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {..});
pub static HOLESKY: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {..});
...
```

## Example

Here is an example for how to build an node with the `NodeBuilder`.

```rust
pub fn get_test_node_config() -> NodeConfig<ChainSpec> {
    NodeConfig::test()
        .dev()
        .with_rpc(RpcServerArgs::default().with_http())
        .with_chain(custom_chain())
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let node_config = get_test_node_config();

    let NodeHandle {
        node,
        node_exit_future: _,
    } = NodeBuilder::new(node_config)
        .testing_node(tasks.executor())
        .with_types::<EthereumNode>()
        .with_components(
            // Note: we use `ParallelExecutorBuilder` here.
            EthereumNode::components().executor(ParallelExecutorBuilder::default()),
        )
        .with_add_ons(EthereumAddOns::default())
        .launch()
        .await
        .unwrap();

    node_exit_future.await
}
```
