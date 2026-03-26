//! IPC throughput benchmark using real multi-process cluster.
//!
//! Starts ABCI servers + hotmint-node processes (with proxy_app),
//! then observes block production via RPC.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hotmint_abci::server::{ApplicationHandler, IpcApplicationServer};
use hotmint_mgmt::cluster;
use hotmint_types::context::OwnedBlockContext;
use hotmint_types::validator_update::EndBlockResponse;
use hotmint_types::*;

const NUM_VALIDATORS: u32 = 4;
const DURATION_SECS: u64 = 10;

struct BenchHandler {
    commit_count: Arc<AtomicU64>,
}

impl ApplicationHandler for BenchHandler {
    fn create_payload(&self, _ctx: OwnedBlockContext) -> Vec<u8> {
        let data = vec![0xABu8; 1024];
        let len = data.len() as u32;
        let mut payload = Vec::with_capacity(4 + data.len());
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(&data);
        payload
    }

    fn execute_block(
        &self,
        _txs: Vec<Vec<u8>>,
        _ctx: OwnedBlockContext,
    ) -> Result<EndBlockResponse, String> {
        Ok(EndBlockResponse::default())
    }

    fn on_commit(&self, _block: Block, _ctx: OwnedBlockContext) -> Result<(), String> {
        self.commit_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

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

#[tokio::main]
async fn main() {
    println!("=== IPC Throughput Benchmark (multi-process, Unix socket, real P2P) ===\n");

    let base_dir = std::env::temp_dir().join(format!("hotmint-bench-ipc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "bench-ipc",
        p2p_base,
        rpc_base,
        "127.0.0.1",
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    // Start IPC servers.
    let commit_count = Arc::new(AtomicU64::new(0));
    let mut sock_paths = Vec::new();
    let mut server_handles = Vec::new();
    for i in 0..NUM_VALIDATORS {
        let path = base_dir.join(format!("app-{i}.sock"));
        let handler = BenchHandler {
            commit_count: commit_count.clone(),
        };
        let server = Arc::new(IpcApplicationServer::new(&path, handler));
        let s = Arc::clone(&server);
        server_handles.push(tokio::spawn(async move {
            let _ = s.run().await;
        }));
        sock_paths.push(path);
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Patch config.toml to set proxy_app for each node.
    for (i, v) in state.validators.iter().enumerate() {
        let config_path = std::path::Path::new(&v.home_dir).join("config/config.toml");
        let config_text = std::fs::read_to_string(&config_path).unwrap();
        let patched = config_text.replace(
            "proxy_app = \"\"",
            &format!("proxy_app = \"{}\"", sock_paths[i].display()),
        );
        std::fs::write(&config_path, patched).unwrap();
    }

    // Build and start hotmint-node processes (which support proxy_app).
    let binary = hotmint_mgmt::build_binary("hotmint", Some("hotmint-node"))
        .expect("failed to build hotmint-node");

    let mut children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &["node"]);

    // Wait for cluster.
    let rpc_port = state.validators[0].rpc_port;
    if !hotmint_mgmt::wait_for_rpc("127.0.0.1", rpc_port, 15) {
        eprintln!("  ERROR: cluster did not start");
        for c in &mut children {
            let _ = c.kill();
        }
        for h in &server_handles {
            h.abort();
        }
        let _ = std::fs::remove_dir_all(&base_dir);
        return;
    }

    std::thread::sleep(Duration::from_secs(2));
    let start_height = query_height("127.0.0.1", rpc_port).unwrap_or(0);
    let start = Instant::now();

    tokio::time::sleep(Duration::from_secs(DURATION_SECS)).await;
    let elapsed = start.elapsed();

    let end_height = query_height("127.0.0.1", rpc_port).unwrap_or(start_height);
    let server_commits = commit_count.load(Ordering::Relaxed);
    let blocks = end_height.saturating_sub(start_height);
    let blocks_per_sec = blocks as f64 / elapsed.as_secs_f64();
    let ms_per_block = if blocks > 0 {
        elapsed.as_millis() as f64 / blocks as f64
    } else {
        f64::INFINITY
    };

    println!(
        "    {NUM_VALIDATORS} validators (separate processes), {DURATION_SECS}s, Unix socket IPC"
    );
    println!("    Result: {blocks_per_sec:.1} blocks/sec, {ms_per_block:.1} ms/block");
    println!(
        "    Total: {blocks} blocks (height {start_height}→{end_height}), {server_commits} server commits"
    );
    println!();

    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    for h in &server_handles {
        h.abort();
    }
    let _ = std::fs::remove_dir_all(&base_dir);

    println!("Done.");
}
