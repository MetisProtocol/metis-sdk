# Metis Ansible
ansible deployment scripts

## LazAI
* hardware requirements (recommendations)
  * CPU: 8 cores
  * Memory: 16 GB
  * Storage: 512 GB SSD [as of `2025-11-13`] (expected 2–3 TB increase per year)
  * Network: 1 Gbps
  * Archive nodes and tracing nodes benefit from faster network bandwidth
* os requirements
  * supported architecture: arm64, amd64
  * supported os: debian, ubuntu
    * for ubuntu / debian (<= 12), change ansible var `lazai_apt_mode` to `apt_repository` (default is `deb822_repository`)
  * lazai_role: default `rpc`, change to `seq` if plan to run in validator mode
  * install ansible (`pip install ansible`) (more details refer to https://docs.ansible.com)
* runbook
1. follow [create validator](../../website/content/docs/architecture/validator/create.mdx)
    * create private key and put rename under `roles/lazai/templates/opt/nodes/mala/config/priv_validator_key.{{ lazai_node_name | "default to $(hostname -s)"}}.json`
    * [optional] contact LazAI (https://lazai.network) to get a most recent data snapshot, alternatively sync from beginning
      * mala - put under `mala/data`
      * reth - put under `reth/rethdata`
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
* ansible vars

**Core lazai variables**
| Variable                   | Default                                       | Description                                                                                                                      |
| -------------------------- | --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `lazai_env`                | **(required, no default)**                    | LazAI network to connect to (e.g. `mainnet`, `testnet`). Used to download the correct `genesis.json` and JWT secret.             |
| `lazai_role`               | `rpc`                                         | Node role. Currently one of `seq` (sequencer) or `rpc` (RPC node). Controls which compose files/templates are used.              |
| `lazai_node_name`          | `inventory_hostname` (first label before `.`) | Logical node name; used to build paths under `/opt/nodes/{{ lazai_node_name }}` and to pick per-node files (e.g. validator key). |
| `lazai_lazai_enabled`      | `true`                                        | Whether to install and configure the LazAI node itself (`lazai.yml`).                                                            |
| `lazai_blockscout_enabled` | `false`                                       | Whether to install Blockscout (explorer) resources for this node.                                                                |

**OS / package management**
| Variable         | Default                                                              | Description                                                                                                                               |
| ---------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `lazai_apt_mode` | `deb822_repository`                                                  | How to configure the Docker APT repo. `deb822_repository` (new style) or `apt_repository` (legacy). Passed to `docker.yml` as `apt_mode`. |
| `lazai_arch`     | Derived: `arm64` if `ansible_architecture == 'aarch64'` else `amd64` | Target CPU architecture, used when downloading `node_exporter` and configuring Docker repo. Normally auto-detected.                       |
| `lazai_os`       | Derived: `ansible_system \| lower` (e.g. `linux`)                    | Target OS name used in download URLs (Docker GPG key, node_exporter tarball). Normally auto-detected.                                     |

**User, paths & directories**
| Variable                | Default                                                                 | Description                                                                                                                |
| ----------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `lazai_user`            | `lazai`                                                                 | System user that owns LazAI files and runs node processes (via scripts / Docker).                                          |
| `lazai_group`           | `lazai`                                                                 | Group for the LazAI user.                                                                                                  |
| `lazai_home`            | `/opt/nodes`                                                            | Base directory where LazAI node data and scripts are placed.                                                               |
| `lazai_bin`             | `/usr/sbin/nologin`                                                     | Login shell for the `lazai` user (usually non-interactive).                                                                |
| `lazai_lazai_dirs`      | List of paths under `{{ lazai_home }}/{{ lazai_node_name }}`            | Directories created for mala/reth data, configs, assets, and logs. You can extend this list if you need extra directories. |
| `lazai_blockscout_dirs` | List of paths under `{{ lazai_home }}/{{ lazai_node_name }}/blockscout` | Directories used when Blockscout support is enabled.                                                                       |

**Docker / images**
| Variable                          | Default                                   | Description                                                                                   |
| --------------------------------- | ----------------------------------------- | --------------------------------------------------------------------------------------------- |
| `lazai_mala_image`                | `ghcr.io/0xlazai/malachitebft:v0.7.1`     | Docker image for the MalachiteBFT (consensus / sequencer) service. Used in compose templates. |
| `lazai_reth_image`                | `ghcr.io/0xlazai/lazchain:v0.7.1`         | Docker image for the execution client (reth / lazchain). Used in compose templates.           |
| `lazai_blockscout_frontend_image` | `ghcr.io/0xlazai/frontend:v2.4.1-1c38a14` | Docker image for Blockscout frontend (if Blockscout is enabled).                              |
| `lazai_blockscout_backend_image`  | `ghcr.io/blockscout/blockscout:8.1.2`     | Docker image for Blockscout backend (if Blockscout is enabled).                               |

**Sysctl / tuning**
| Variable               | Default              | Description                                                                                             |
| ---------------------- | -------------------- | ------------------------------------------------------------------------------------------------------- |
| `lazai_sysctl_enabled` | `true`               | Whether to apply kernel tuning via `sysctl` (`sysctl.yml`).                                             |
| `lazai_sysctl`         | (dict, values below) | Map of sysctl keys → values written into `/etc/sysctl.d/lazai.conf` and applied with `sysctl --system`. |

lazai_sysctl keys default value:
| Key                            | Default      | Description                                                        |
| ------------------------------ | ------------ | ------------------------------------------------------------------ |
| `net.core.rmem_default`        | `134217728`  | Default receive buffer size for sockets.                           |
| `net.core.rmem_max`            | `134217728`  | Max receive buffer size for sockets.                               |
| `net.core.wmem_default`        | `134217728`  | Default send buffer size.                                          |
| `net.core.wmem_max`            | `134217728`  | Max send buffer size.                                              |
| `vm.max_map_count`             | `2024000000` | Max memory map areas per process (useful for DB / node workloads). |
| `fs.nr_open`                   | `2147483584` | Max number of file descriptors that can be allocated system-wide.  |
| `fs.aio-max-nr`                | `8388608`    | Max number of concurrent AIO operations.                           |
| `net.core.somaxconn`           | `524288`     | Max listen backlog size.                                           |
| `net.ipv4.ip_local_port_range` | `1024 65535` | Ephemeral port range for outgoing connections.                     |
| `net.ipv4.tcp_tw_reuse`        | `2`          | TIME_WAIT reuse behavior (aggressive reuse on modern kernels).     |
| `net.ipv4.tcp_timestamps`      | `1`          | Enable TCP timestamps.                                             |
| `net.ipv4.tcp_max_syn_backlog` | `524288`     | Max SYN backlog queue size.                                        |
| `net.core.netdev_max_backlog`  | `524288`     | Max packets backlog per network interface.                         |

**Firewall / networking**
| Variable            | Default | Description                                                                                      |
| ------------------- | ------- | ------------------------------------------------------------------------------------------------ |
| `lazai_ufw_enabled` | `true`  | Whether to configure UFW using `ufw.yml` (allow SSH/HTTP/HTTPS and required ports 30303, 27000). |

(UFW rules themselves are hard-coded in ufw.yml, not parameterised yet.)

**Monitoring (node_exporter)**
| Variable                      | Default                                                   | Description                                                              |
| ----------------------------- | --------------------------------------------------------- | ------------------------------------------------------------------------ |
| `lazai_node_exporter_enabled` | `true`                                                    | Whether to install and configure Prometheus `node_exporter`.             |
| `lazai_node_exporter_version` | `1.9.1`                                                   | node_exporter release version to download.                               |
| `lazai_node_exporter_args`    | `--web.listen-address 127.0.0.1:9100 --collector.systemd` | Extra CLI args passed to node_exporter via the systemd service template. |

**Sequencer / peer config**
| Variable           | Default     | Description                                                                                    |
| ------------------ | ----------- | ---------------------------------------------------------------------------------------------- |
| `lazai_sequencers` | (see below) | List of sequencer definitions used in mala config and compose templates as peers / validators. |

Each entry in lazai_sequencers has:
| Key    | Description                                                                              |
| ------ | ---------------------------------------------------------------------------------------- |
| `name` | Human-readable name / hostname of the sequencer (e.g. `pub.rpc0.mainnet.lazai.systems`). |
| `id`   | Sequencer / validator ID or public key used by the consensus engine.                     |
| `addr` | IP or hostname of the sequencer peer.                                                    |
| `port` | P2P port for the sequencer (e.g. `30303`).                                               |
