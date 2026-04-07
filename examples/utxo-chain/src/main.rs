//! UTXO chain demo using a real multi-process cluster.

use std::time::Duration;

use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;

fn main() {
    println!("=== Hotmint UTXO Chain Demo (multi-process) ===\n");
    println!("NOTE: UTXO logic runs inside cluster-node (NoopApplication).");
    println!("      Full UTXO node with --home support is a future enhancement.\n");

    let binary = hotmint_mgmt::build_binary("cluster-node", Some("cluster-node"))
        .expect("failed to build cluster-node");

    let base_dir = std::env::temp_dir().join(format!("hotmint-utxo-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "utxo-demo",
        p2p_base,
        rpc_base,
        hotmint_mgmt::loopback_addr(),
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    let mut children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &[]);
    for (i, c) in children.iter().enumerate() {
        println!("  V{}: started (pid {})", state.validators[i].id, c.id());
    }

    let rpc_port = state.validators[0].rpc_port;
    if !hotmint_mgmt::wait_for_rpc(hotmint_mgmt::loopback_addr(), rpc_port, 15) {
        eprintln!("ERROR: cluster did not start");
    }

    println!("\nCluster running for 30s...\n");
    std::thread::sleep(Duration::from_secs(30));

    println!("=== UTXO Chain Demo Complete ===");
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);
}
