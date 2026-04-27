//! Integration test: 4-node consensus via real multi-process cluster.

use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;

fn query_height(host: &str, port: u16) -> Option<u64> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let addr = hotmint_mgmt::format_host_port(host, port);
    let mut stream =
        TcpStream::connect_timeout(&addr.parse().ok()?, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"status","params":[]}"#;
    stream.write_all(req.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;

    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).ok()?;
    let text = std::str::from_utf8(&buf[..n]).ok()?;
    let val: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    val["result"]["last_committed_height"].as_u64()
}

use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn serial_test_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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

    // Use unique chain_id per test to prevent PEX cross-cluster discovery
    // when multiple tests run in parallel on the same machine.
    let chain_id = format!("test-four-node-{}-{id}", std::process::id());
    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        &chain_id,
        p2p_base,
        rpc_base,
        hotmint_mgmt::loopback_addr(),
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    let children = hotmint_mgmt::start_cluster_nodes(&binary, &state, &base_dir, &[]);

    (children, state, base_dir)
}

#[test]
fn test_four_node_consensus_commits_blocks() {
    let _guard = serial_test_guard();
    let (mut children, state, base_dir) = setup_cluster();
    // Wait for ALL nodes to be ready before measuring.
    for v in &state.validators {
        assert!(
            hotmint_mgmt::wait_for_rpc(hotmint_mgmt::loopback_addr(), v.rpc_port, 20),
            "V{} did not start within 20s",
            v.id
        );
    }

    // Wait for blocks to accumulate.
    std::thread::sleep(Duration::from_secs(8));

    // All validators should have committed blocks.
    for v in &state.validators {
        let height = query_height(hotmint_mgmt::loopback_addr(), v.rpc_port).unwrap_or(0);
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
        .filter_map(|v| query_height(hotmint_mgmt::loopback_addr(), v.rpc_port))
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
    let _guard = serial_test_guard();
    let (mut children, state, base_dir) = setup_cluster();

    // Wait for ALL nodes to be ready before killing one.
    for v in &state.validators {
        assert!(
            hotmint_mgmt::wait_for_rpc(hotmint_mgmt::loopback_addr(), v.rpc_port, 20),
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
        let height = query_height(hotmint_mgmt::loopback_addr(), v.rpc_port).unwrap_or(0);
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
