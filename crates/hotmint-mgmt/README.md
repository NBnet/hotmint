# hotmint-mgmt

Cluster management library for [Hotmint](https://github.com/rust-util-collections/hotmint).

A reusable library for initializing, starting, stopping, and deploying multi-node Hotmint clusters — both locally and on remote machines via SSH.

## Library API

| Function | Description |
|:---------|:------------|
| `find_free_ports(n)` | Allocate N ephemeral TCP ports |
| `build_binary(package, bin_name)` | Build a workspace crate in release mode |
| `start_node_process(binary, home, log)` | Spawn a single node process |
| `start_cluster_nodes(binary, state, base_dir, extra_args)` | Start all validators with staggered startup |
| `wait_for_rpc(host, port, timeout)` | Poll until RPC endpoint responds |
| `kill_stale_nodes(base_dir)` | Kill orphaned node processes from previous runs |

### Modules

| Module | Description |
|:-------|:------------|
| `cluster` | `init_cluster()`, `clean()`, `destroy()`, `info()`, `ClusterState` |
| `local` | `start()`, `stop()`, `status()` — PID-based local process management |
| `remote` | `deploy()`, `exec_all()`, `push_all()`, `pull_all()`, `logs()` — SSH-based remote ops |

## License

GPL-3.0-only
