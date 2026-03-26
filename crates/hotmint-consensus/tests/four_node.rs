//! Integration test: 4-node consensus via real multi-process cluster.

use std::process::Child;
use std::time::Duration;

use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;

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

use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn setup_cluster() -> (Vec<Child>, cluster::ClusterState, std::path::PathBuf) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let base_dir =
        std::env::temp_dir().join(format!("hotmint-four-node-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let binary = hotmint_mgmt::build_binary("cluster-node", Some("cluster-node"))
        .expect("failed to build cluster-node");

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "test-four-node",
        p2p_base,
        rpc_base,
        "127.0.0.1",
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    let children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &[]);

    (children, state, base_dir)
}

#[test]
fn test_four_node_consensus_commits_blocks() {
    let (mut children, state, base_dir) = setup_cluster();
    let rpc_port = state.validators[0].rpc_port;

    // Wait for cluster to start.
    assert!(
        hotmint_mgmt::wait_for_rpc("127.0.0.1", rpc_port, 20),
        "cluster did not start within 20s"
    );

    // Wait for blocks to accumulate.
    std::thread::sleep(Duration::from_secs(8));

    // All validators should have committed blocks.
    for v in &state.validators {
        let height = query_height("127.0.0.1", v.rpc_port).unwrap_or(0);
        assert!(
            height >= 1,
            "V{} committed {} blocks, expected >= 1",
            v.id,
            height
        );
    }

    // At least one validator should have committed multiple blocks.
    let max_height = state
        .validators
        .iter()
        .filter_map(|v| query_height("127.0.0.1", v.rpc_port))
        .max()
        .unwrap_or(0);
    assert!(max_height >= 2, "max height is {max_height}, expected >= 2");

    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);
}

#[test]
fn test_consensus_tolerates_one_silent_validator() {
    let (mut children, state, base_dir) = setup_cluster();

    // Wait for ALL nodes to be ready before killing one.
    for v in &state.validators {
        assert!(
            hotmint_mgmt::wait_for_rpc("127.0.0.1", v.rpc_port, 20),
            "V{} did not start within 20s",
            v.id
        );
    }

    // Let the cluster run briefly so all connections are established.
    std::thread::sleep(Duration::from_secs(3));

    // Kill validator 3 — simulate a crashed node.
    let _ = children[3].kill();
    let _ = children[3].wait();

    // With 3 of 4 validators (quorum=3), consensus should still work.
    std::thread::sleep(Duration::from_secs(10));

    // The first 3 active validators should commit blocks.
    for v in state.validators.iter().take(3) {
        let height = query_height("127.0.0.1", v.rpc_port).unwrap_or(0);
        assert!(
            height >= 1,
            "active V{} committed {} blocks, expected >= 1",
            v.id,
            height
        );
    }

    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let _ = std::fs::remove_dir_all(&base_dir);
}
