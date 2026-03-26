//! hotmint-demo: Start a 4-node cluster using hotmint-mgmt, observe blocks.
//!
//! Usage:
//!   cargo run -p hotmint-demo
//!   (builds cluster-node, inits a temp cluster, starts nodes, monitors for 30s)

use std::time::{Duration, Instant};

use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;
const DURATION_SECS: u64 = 30;

fn main() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("hotmint-demo: 4-node multi-process consensus demo");
        println!("Usage: hotmint-demo");
        println!();
        println!(
            "Runs {} validators as separate processes for {} seconds.",
            NUM_VALIDATORS, DURATION_SECS
        );
        return;
    }

    println!("=== Hotmint Demo ({NUM_VALIDATORS} validators, {DURATION_SECS}s) ===\n");

    // Build cluster-node binary.
    println!("Building cluster-node...");
    let binary = hotmint_mgmt::build_binary("cluster-node", Some("cluster-node"))
        .expect("failed to build cluster-node");
    println!("  Binary: {}\n", binary.display());

    // Init cluster in temp dir.
    let base_dir = std::env::temp_dir().join(format!("hotmint-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "hotmint-demo",
        p2p_base,
        rpc_base,
        "127.0.0.1",
    )
    .expect("init cluster");

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    // Start all nodes with staggered startup.
    let mut children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &[]);
    for (i, c) in children.iter().enumerate() {
        let v = &state.validators[i];
        println!(
            "  V{}: started (pid {}, p2p={}, rpc={})",
            v.id,
            c.id(),
            v.p2p_port,
            v.rpc_port
        );
    }

    println!("\nAll validators spawned, monitoring consensus...\n");

    // Monitor via RPC for DURATION_SECS.
    let start = Instant::now();
    let rpc_port = state.validators[0].rpc_port;

    // Wait for first RPC response.
    if !hotmint_mgmt::wait_for_rpc("127.0.0.1", rpc_port, 15) {
        eprintln!("ERROR: cluster did not start within 15s");
    }

    while start.elapsed() < Duration::from_secs(DURATION_SECS) {
        std::thread::sleep(Duration::from_secs(3));
        // Query each node's status via RPC.
        for v in &state.validators {
            if let Some(info) = query_height("127.0.0.1", v.rpc_port) {
                println!("  V{}: height={info}", v.id);
            }
        }
    }

    println!("\n=== Demo complete ({DURATION_SECS}s) ===");

    // Kill all.
    for child in &mut children {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);
}

fn query_height(host: &str, port: u16) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let addr = format!("{host}:{port}");
    let mut stream =
        TcpStream::connect_timeout(&addr.parse().ok()?, Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"status","params":[]}"#;
    stream.write_all(req.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).ok()?;
    let text = std::str::from_utf8(&buf[..n]).ok()?;
    let val: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let h = val["result"]["last_committed_height"].as_u64()?;
    let v = val["result"]["current_view"].as_u64()?;
    Some(format!("{h}, view={v}"))
}
