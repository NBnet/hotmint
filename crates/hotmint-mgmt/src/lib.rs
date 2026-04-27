//! hotmint-mgmt library: cluster initialization, lifecycle, and deployment.
//!
//! Exposes the core cluster management logic as a reusable library so that
//! benchmarks, tests, and custom tooling can programmatically create and
//! manage multi-node Hotmint clusters.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use hotmint_mgmt::cluster;
//!
//! let base_dir = std::path::Path::new("/tmp/my-cluster");
//! let ip = hotmint_mgmt::loopback_addr();
//! cluster::init_cluster(base_dir, 4, "test-chain", 20000, 21000, ip).unwrap();
//!
//! let state = cluster::ClusterState::load(base_dir).unwrap();
//! // state.validators contains port info for RPC connections
//! ```

pub mod cluster;
pub mod local;
pub mod remote;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// Detect the available loopback address (IPv4 preferred, IPv6 fallback).
///
/// Returns `"127.0.0.1"` on most systems. On hosts where the loopback
/// interface has no IPv4 address (e.g. FreeBSD jails with IPv6-only lo0)
/// returns `"::1"`.
pub fn loopback_addr() -> &'static str {
    use std::sync::OnceLock;
    static ADDR: OnceLock<&str> = OnceLock::new();
    ADDR.get_or_init(|| {
        if std::net::TcpListener::bind("127.0.0.1:0").is_ok() {
            "127.0.0.1"
        } else {
            "::1"
        }
    })
}

/// Format a host:port pair as a socket address string.
///
/// IPv6 addresses are wrapped in brackets: `[::1]:8080`.
pub fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Find N free TCP ports on the loopback interface by binding to port 0.
///
/// Returns the ports after releasing the listeners. There is a small
/// race window, but it is acceptable for tests and benchmarks.
pub fn find_free_ports(n: usize) -> Vec<u16> {
    let bind = format_host_port(loopback_addr(), 0);
    let mut ports = Vec::with_capacity(n);
    let mut listeners = Vec::with_capacity(n);
    for _ in 0..n {
        let listener = std::net::TcpListener::bind(&bind).expect("bind to ephemeral port");
        ports.push(listener.local_addr().unwrap().port());
        listeners.push(listener);
    }
    drop(listeners);
    ports
}

/// Build a workspace crate in release mode and return the path to the binary.
///
/// This is the pattern used by tests that need to spawn node processes
/// (e.g., `cluster-node`, `hotmint-evm`).
///
/// Returns `None` if the build fails.
pub fn build_binary(package: &str, bin_name: Option<&str>) -> Option<PathBuf> {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--release", "-p", package]);
    if let Some(name) = bin_name {
        cmd.args(["--bin", name]);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    match cmd.status() {
        Ok(s) if s.success() => {
            // Find the binary in target/release
            let name = bin_name.unwrap_or(package);
            let workspace_root = find_workspace_root()?;
            let binary = workspace_root.join("target/release").join(name);
            if binary.exists() { Some(binary) } else { None }
        }
        _ => None,
    }
}

/// Locate the workspace root by walking up from CARGO_MANIFEST_DIR.
fn find_workspace_root() -> Option<PathBuf> {
    // Try using `cargo metadata` for accuracy
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Quick parse: look for "workspace_root":"..."
    let prefix = "\"workspace_root\":\"";
    let start = text.find(prefix)? + prefix.len();
    let end = text[start..].find('"')? + start;
    Some(PathBuf::from(&text[start..end]))
}

/// Start a cluster node process with the given binary and home directory.
///
/// Returns the `Child` process handle. Stdout/stderr are piped to a log file
/// at `log_path` if provided, otherwise inherited.
pub fn start_node_process(
    binary: &Path,
    home_dir: &Path,
    log_path: Option<&Path>,
) -> std::io::Result<Child> {
    let mut cmd = Command::new(binary);
    cmd.arg("--home").arg(home_dir);

    if let Some(log) = log_path {
        let log_file = std::fs::File::create(log)?;
        let log_err = log_file.try_clone()?;
        cmd.stdout(log_file).stderr(log_err);
    }

    cmd.spawn()
}

/// Wait until an RPC endpoint responds, with a timeout.
///
/// Tries a raw TCP connection + JSON-RPC `status` query.
pub fn wait_for_rpc(host: &str, port: u16, timeout_secs: u64) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Instant;

    let addr = format_host_port(host, port);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    while Instant::now() < deadline {
        if let Ok(mut stream) =
            TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_secs(1))
        {
            stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
            let req = r#"{"jsonrpc":"2.0","id":1,"method":"status","params":[]}"#;
            if stream.write_all(req.as_bytes()).is_ok() && stream.write_all(b"\n").is_ok() {
                let mut buf = vec![0u8; 4096];
                if let Ok(n) = stream.read(&mut buf)
                    && n > 0
                {
                    return true;
                }
            }
        }
        sleep(Duration::from_millis(200));
    }
    false
}

/// Kill any stale node processes whose `--home` points into `base_dir`.
///
/// Uses only PID files under `base_dir`, validates the live process command
/// line, and signals that exact PID. This cleans up orphaned nodes from
/// previous test runs that crashed without risking broad process matches.
pub fn kill_stale_nodes(base_dir: &Path) {
    let Ok(entries) = fs::read_dir(base_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let pid_file = entry.path();
        let Some(file_name) = pid_file.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(id) = file_name
            .strip_prefix('v')
            .and_then(|name| name.strip_suffix(".pid"))
            .and_then(|id| id.parse::<u64>().ok())
        else {
            continue;
        };
        let Some(pid) = read_pid_file(&pid_file) else {
            let _ = fs::remove_file(&pid_file);
            continue;
        };
        let home_dir = base_dir.join(format!("v{id}"));
        if is_expected_node_process(pid, &home_dir) {
            let _ = Command::new("kill")
                .args(["-9", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = fs::remove_file(&pid_file);
    }

    // Brief pause to let OS reclaim resources.
    sleep(Duration::from_millis(100));
}

fn read_pid_file(path: &Path) -> Option<u32> {
    let pid = fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()?;
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    Some(pid)
}

fn process_command_line(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!command.is_empty()).then_some(command)
}

fn process_name(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

fn is_expected_node_process(pid: u32, home_dir: &Path) -> bool {
    let Some(name) = process_name(pid) else {
        return false;
    };
    let executable_ok = name.contains("cluster-node") || name.contains("hotmint");
    if !executable_ok {
        return false;
    }

    let Some(command) = process_command_line(pid) else {
        return false;
    };
    command.contains("--home") && command.contains(home_dir.to_string_lossy().as_ref())
}

/// Start cluster node processes with staggered startup to avoid
/// simultaneous Noise handshake collisions in litep2p.
///
/// `args_fn` can optionally provide extra CLI arguments per validator
/// (e.g., `["node"]` for hotmint-node subcommand).
pub fn start_cluster_nodes(
    binary: &Path,
    state: &cluster::ClusterState,
    base_dir: &Path,
    extra_args: &[&str],
) -> Vec<Child> {
    // Clean up orphaned nodes from previous runs.
    kill_stale_nodes(base_dir);

    let mut children = Vec::new();
    for (i, v) in state.validators.iter().enumerate() {
        let log = std::fs::File::create(base_dir.join(format!("v{}.log", v.id)))
            .expect("create log file");
        let log_err = log.try_clone().expect("clone log file");
        let mut cmd = Command::new(binary);
        for arg in extra_args {
            cmd.arg(arg);
        }
        cmd.arg("--home").arg(&v.home_dir);
        cmd.stdout(log).stderr(log_err);
        let child = cmd.spawn().expect("spawn node process");
        if let Err(e) = fs::write(
            base_dir.join(format!("v{}.pid", v.id)),
            child.id().to_string(),
        ) {
            eprintln!("WARNING: failed to write pid file for V{}: {}", v.id, e);
        }
        children.push(child);

        // Stagger startup to avoid simultaneous Noise handshake collisions.
        if i < state.validators.len() - 1 {
            sleep(Duration::from_millis(300));
        }
    }
    children
}
