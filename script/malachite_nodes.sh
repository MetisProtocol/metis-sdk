#!/usr/bin/env bash

#start the malaketh-layer nodes

cd ../lib/malaketh-layered
cargo run --release --bin malachitebft-eth-app -- testnet --nodes 3 --home nodes
echo 👉 Grafana dashboard is available at http://localhost:3000
bash scripts/spawn.bash --nodes 3 --home nodes
cd -
