# Guide

## Necessary Modifications

### 1. configs/node/config/config.toml
Update `persistent_peers` under `[consensus.p2p]` to the IP addresses of the peers in the target chain you want to connect to.

### 2. genesis.json
Replace with the `genesis.json` file corresponding to the target chain you want to connect to.
need to be replaced file: 
configs/node/config/genesis.json
configs/assets/genesis.json

### 3. configs/node/config/priv_validator_key.json
You need to generate your own private key and replace this file. Steps to generate the file:
```bash
git clone https://github.com/MetisProtocol/malaketh-layered.git
cd malaketh-layered
cargo run --bin malachitebft-eth-app -- testnet --nodes 1 --home nodes
ls nodes/0/config/priv_validator_key.json
```

### 4. configs/assets/jwtsecret
Replace with the `jwtsecret` file corresponding to the target chain you want to connect to.

### 5. compose.yaml
Replace the "bootnodes" value to the target chain you want to connect to

## Optional Modifications

### 1. compose.yaml
1. Service name
2. `--http.port`: The port used to provide the engine service
3. `--authrpc.port`: The port used to provide the RPC service
4. `volumes` mapping: The `./configs` directory must match the directory where your configuration files are actually stored

### 2. configs/node/config/app_config.toml
1. Ensure that `reth0` in this file matches the service name `reth0` in `compose.yaml`
2. The port in `engine_url` must match `--authrpc.port` in `compose.yaml`
3. The port in `eth_url` must match `--http.port` in `compose.yaml`

## After completing the above modifications, start the service with Docker
```bash
./start_node.sh
```
