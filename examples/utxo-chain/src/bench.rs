//! UTXO throughput benchmark using a real multi-process cluster.

use std::time::{Duration, Instant};

use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;
const DURATION_SECS: u64 = 10;

fn query_height(host: &str, port: u16) -> Option<u64> {
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
    val["result"]["last_committed_height"].as_u64()
}

fn main() {
    println!("=== UTXO Throughput Benchmark (multi-process, real P2P) ===\n");
    println!("NOTE: Uses cluster-node (NoopApp). Full UTXO execution benchmark");
    println!("      requires a UTXO node binary with --home support.\n");

    let binary = hotmint_mgmt::build_binary("cluster-node", Some("cluster-node"))
        .expect("failed to build cluster-node");

    let base_dir = std::env::temp_dir().join(format!("hotmint-utxo-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    // Initialize vsdb for the bench process itself (if needed by UTXO lib).
    let vsdb_dir = base_dir.join("vsdb");
    let _ = std::fs::create_dir_all(&vsdb_dir);
    vsdb::vsdb_set_base_dir(&vsdb_dir).unwrap();

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "utxo-bench",
        p2p_base,
        rpc_base,
        "127.0.0.1",
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    let mut children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &[]);

    let rpc_port = state.validators[0].rpc_port;
    if !hotmint_mgmt::wait_for_rpc("127.0.0.1", rpc_port, 15) {
        eprintln!("  ERROR: cluster did not start");
        for c in &mut children {
            let _ = c.kill();
        }
        let _ = std::fs::remove_dir_all(&base_dir);
        return;
    }

    std::thread::sleep(Duration::from_secs(2));
    let start_height = query_height("127.0.0.1", rpc_port).unwrap_or(0);
    let start = Instant::now();

    std::thread::sleep(Duration::from_secs(DURATION_SECS));

    let elapsed = start.elapsed();
    let end_height = query_height("127.0.0.1", rpc_port).unwrap_or(start_height);
    let blocks = end_height.saturating_sub(start_height);
    let blocks_per_sec = blocks as f64 / elapsed.as_secs_f64();
    let ms_per_block = if blocks > 0 {
        elapsed.as_millis() as f64 / blocks as f64
    } else {
        f64::INFINITY
    };

    println!("    {NUM_VALIDATORS} validators (separate processes), {DURATION_SECS}s");
    println!(
        "    Result: {blocks_per_sec:.1} blocks/sec, {ms_per_block:.1} ms/block, {blocks} blocks"
    );
    println!();

    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);

    println!("Done.");
}
