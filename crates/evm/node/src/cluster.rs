//! EVM-specific cluster management helpers.
//!
//! Wraps the generic `hotmint_mgmt` framework with EVM-specific logic:
//! - Allocates extra ports for Ethereum JSON-RPC
//! - Writes `evm-genesis.json` to each node's config directory
//! - Starts `hotmint-evm` processes with `--rpc-addr`

use std::path::Path;
use std::process::{Child, Command};

use hotmint_evm_types::genesis::EvmGenesis;
use hotmint_mgmt::cluster::{self, ClusterState};

/// Initialize an EVM cluster: framework init + evm-genesis.json + eth RPC ports.
///
/// Returns `(cluster_state, eth_rpc_ports)` where `eth_rpc_ports[i]` is the
/// Ethereum JSON-RPC port for validator `i`.
pub fn init_evm_cluster(
    base_dir: &Path,
    validator_count: u32,
    chain_id: &str,
    evm_genesis: &EvmGenesis,
    bind_ip: &str,
) -> ruc::Result<(ClusterState, Vec<u16>)> {
    // 3 ports per validator: p2p, consensus RPC, eth RPC.
    let ports = hotmint_mgmt::find_free_ports((validator_count * 3) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[validator_count as usize];
    let eth_rpc_ports: Vec<u16> = ports[(validator_count * 2) as usize..].to_vec();

    cluster::init_cluster(
        base_dir,
        validator_count,
        chain_id,
        p2p_base,
        rpc_base,
        bind_ip,
    )?;

    // Write EVM genesis to each node's config dir.
    let evm_genesis_json = serde_json::to_string_pretty(evm_genesis).map_err(|e| ruc::eg!(e))?;
    for i in 0..validator_count {
        let config_dir = base_dir.join(format!("v{i}")).join("config");
        std::fs::write(config_dir.join("evm-genesis.json"), &evm_genesis_json)
            .map_err(|e| ruc::eg!(e))?;
    }

    let state = ClusterState::load(base_dir)?;
    Ok((state, eth_rpc_ports))
}

/// Start EVM node processes with staggered startup.
///
/// Each node gets `--rpc-addr 127.0.0.1:{eth_rpc_ports[i]}` for Ethereum JSON-RPC.
pub fn start_evm_nodes(
    binary: &Path,
    state: &ClusterState,
    base_dir: &Path,
    eth_rpc_ports: &[u16],
) -> Vec<Child> {
    let mut children = Vec::new();
    for (i, v) in state.validators.iter().enumerate() {
        let log = std::fs::File::create(base_dir.join(format!("v{}.log", v.id)))
            .expect("create log file");
        let log_err = log.try_clone().expect("clone log file");
        let child = Command::new(binary)
            .arg("--home")
            .arg(&v.home_dir)
            .arg("--rpc-addr")
            .arg(format!("127.0.0.1:{}", eth_rpc_ports[i]))
            .stdout(log)
            .stderr(log_err)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn V{}: {e}", v.id));
        children.push(child);

        // Stagger startup to avoid simultaneous Noise handshake collisions.
        if i < state.validators.len() - 1 {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }
    children
}
