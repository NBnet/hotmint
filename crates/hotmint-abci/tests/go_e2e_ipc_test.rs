//! E2E test: Go ABCI servers + real multi-process hotmint-node cluster.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use hotmint_abci::client::IpcApplicationClient;
use hotmint_consensus::application::Application;
use hotmint_mgmt::cluster;

const NUM_VALIDATORS: u32 = 4;

fn build_go_testserver() -> Option<std::path::PathBuf> {
    let go_server_dir = std::env::current_dir().unwrap().join("../../sdk/go");
    let binary = std::env::temp_dir().join(format!("hotmint-go-testserver-{}", std::process::id()));

    let status = Command::new("go")
        .args(["build", "-o", binary.to_str().unwrap(), "./cmd/testserver"])
        .current_dir(&go_server_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status();

    match status {
        Ok(s) if s.success() => Some(binary),
        Ok(_) => {
            eprintln!("go build failed, skipping Go e2e test");
            None
        }
        Err(e) => {
            eprintln!("go not available ({e}), skipping Go e2e test");
            None
        }
    }
}

fn start_go_server(binary: &std::path::Path, sock_path: &std::path::Path) -> Child {
    Command::new(binary)
        .arg(sock_path.to_str().unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start Go server")
}

fn wait_for_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("Go server did not create socket at {:?}", path);
}

/// End-to-end test: hotmint-node processes talking through Go ABCI servers.
///
/// This test requires Go to be installed.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn go_ipc_consensus_e2e() {
    let go_binary = match build_go_testserver() {
        Some(b) => b,
        None => return,
    };

    let base_dir = std::env::temp_dir().join(format!("hotmint-go-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base_dir);

    let ports = hotmint_mgmt::find_free_ports((NUM_VALIDATORS * 2) as usize);
    let p2p_base = ports[0];
    let rpc_base = ports[NUM_VALIDATORS as usize];

    cluster::init_cluster(
        &base_dir,
        NUM_VALIDATORS,
        "go-ipc-e2e",
        p2p_base,
        rpc_base,
        hotmint_mgmt::loopback_addr(),
    )
    .unwrap();

    let state = cluster::ClusterState::load(&base_dir).unwrap();

    // Start Go ABCI servers.
    let mut go_servers: Vec<Child> = Vec::new();
    let mut sock_paths = Vec::new();
    for i in 0..NUM_VALIDATORS {
        let path = base_dir.join(format!("go-app-{i}.sock"));
        let child = start_go_server(&go_binary, &path);
        go_servers.push(child);
        sock_paths.push(path);
    }

    for path in &sock_paths {
        wait_for_socket(path);
    }

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
    let node_binary = hotmint_mgmt::build_binary("hotmint", Some("hotmint-node"))
        .expect("failed to build hotmint-node");

    let mut children =
        hotmint_mgmt::start_cluster_nodes(&node_binary, &state, &base_dir, &["node"]);

    let rpc_port = state.validators[0].rpc_port;
    assert!(
        hotmint_mgmt::wait_for_rpc(hotmint_mgmt::loopback_addr(), rpc_port, 20),
        "cluster did not start within 20s"
    );

    // Run for 5 seconds.
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Stop consensus engines first so IPC connections are released.
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Query commit count from one Go server via IPC.
    let client = IpcApplicationClient::new(&sock_paths[0]);
    let count_bytes = client.query("commits", &[]).unwrap();
    let commits = u64::from_le_bytes(count_bytes.data.try_into().unwrap_or([0; 8]));

    for mut child in go_servers {
        let _ = child.kill();
        let _ = child.wait();
    }

    assert!(
        commits >= 1,
        "expected at least 1 commit via Go IPC, got {commits}"
    );
    eprintln!("Go e2e: {commits} commits in 5 seconds");

    let _ = std::fs::remove_dir_all(&base_dir);
    let _ = std::fs::remove_file(&go_binary);
}
