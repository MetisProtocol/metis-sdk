# LazAI Mainnet (Compose)
compose deployment

## Start Node

1. follow [create validator](../../../../website/content/docs/architecture/validator/create.mdx)
    * create private key and put `priv_validator_key.json` rename under `./mala/data/config`
    * [optional] contact LazAI (https://lazai.network) to get a most recent data snapshot, alternatively sync from beginning
      * mala - put under `mala/data`
      * reth - put under `reth/rethdata`

2. [optional] rename `changethis` in [config.toml](./mala/data/config/config.toml#1)

3. run compose

```bash
# initialize assets
./init.sh
# start
./start.sh
```
