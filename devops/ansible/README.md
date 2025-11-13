# Metis Ansible
ansible deployment scripts

## LazAI

* requirements
  * supported architecture: arm64
  * supported os: debian, ubuntu
    * for ubuntu / debian ( < 12), change ansible var `lazai_apt_mode` to `apt_repository`
  * lazai_role: default `rpc`, change to `seq` if plan to run in validator mode
* runbook
1. follow [create validator](../../website/content/docs/architecture/validator/crate.mdx)
    * create private key and put rename under `roles/lazai/templates/opt/nodes/mala/config/priv_validator_key.{{ lazai_node_name | "default to $(hostname -s)"}}.json`
2. run ansible playbook
```bash
ansible-playbook \
  -i "t{{ inventory_hostname }}," \
  -e "ansible_host={{ ansible_host }} ansible_user={{ ansible_user }} lazai_env={{ lazai_env }}" \
  lazai.yml
```
e.g
```bash
ansible-playbook \
  -i "t0.dev.lazai.systems," \
  -e "ansible_host=1.2.3.4 ansible_user=lazai lazai_env=mainnet" \
  lazai.yml
```
3. check lazai (reth) rpc
```bash
# ssh to host
curl -s -X POST http://localhost:8545/ \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```
