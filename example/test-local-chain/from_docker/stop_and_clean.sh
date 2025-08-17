#!/usr/bin/env bash

# clean metis process and data 
docker compose -f compose-mala.yaml down
docker compose down

rm -fr ./nodes
# Preserving reth0 discovery-secret files and cleaning rethdata...
# reth0 is bootnode
if [ -d "./rethdata/0" ]; then
    mv ./rethdata/0/discovery-secret ./discovery-secret-0
    rm -fr ./rethdata
    mkdir -p ./rethdata/0
    mv ./discovery-secret-0 ./rethdata/0/discovery-secret
fi
