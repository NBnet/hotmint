//! E2E test: Rust ABCI servers + real multi-process hotmint-node cluster.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use hotmint_abci::server::{ApplicationHandler, IpcApplicationServer};
use hotmint_mgmt::cluster;
use hotmint_types::context::OwnedBlockContext;
use hotmint_types::validator_update::EndBlockResponse;
use hotmint_types::*;

const NUM_VALIDATORS: u32 = 4;

struct CommitCounter {
    commit_count: Arc<AtomicU64>,
}

impl ApplicationHandler for CommitCounter {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn ipc_consensus_e2e() {
    let base_dir = std::env::temp_dir().join(format!("hotmint-ipc-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "ipc-e2e",
        p2p_base,
        rpc_base,
        "127.0.0.1",
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    // Start IPC servers in-process.
    let commit_count = Arc::new(AtomicU64::new(0));
    let mut sock_paths = Vec::new();
    let mut server_handles = Vec::new();
    for i in 0..NUM_VALIDATORS {
        let path = base_dir.join(format!("app-{i}.sock"));
        let handler = CommitCounter {
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

    // Build and start hotmint-node processes.
    let binary = hotmint_mgmt::build_binary("hotmint", Some("hotmint-node"))
        .expect("failed to build hotmint-node");

    let mut children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &["node"]);

    let rpc_port = state.validators[0].rpc_port;
    assert!(
        hotmint_mgmt::wait_for_rpc("127.0.0.1", rpc_port, 20),
        "cluster did not start within 20s"
    );

    // Run for 5 seconds.
    tokio::time::sleep(Duration::from_secs(5)).await;

    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    for h in &server_handles {
        h.abort();
    }

    let commits = commit_count.load(Ordering::Relaxed);
    assert!(
        commits >= 1,
        "expected at least 1 commit via IPC, got {commits}"
    );
    eprintln!("Rust IPC e2e: {commits} commits in 5 seconds");

    let _ = std::fs::remove_dir_all(&base_dir);
}
