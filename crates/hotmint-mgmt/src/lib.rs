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

use ruc::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
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
    if host.contains(':') && !host.starts_with('[') {
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
    build_binary_with_cargo(Path::new("cargo"), package, bin_name)
}

fn build_binary_with_cargo(cargo: &Path, package: &str, bin_name: Option<&str>) -> Option<PathBuf> {
    let mut cmd = Command::new(cargo);
    cmd.args(["build", "--release", "-p", package]);
    if let Some(name) = bin_name {
        cmd.args(["--bin", name]);
    }
    // output() drains stdout and stderr concurrently while cargo is running.
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let binary = cargo_target_directory(cargo)?
        .join("release")
        .join(bin_name.unwrap_or(package));
    binary.is_file().then_some(binary)
}

/// Resolve Cargo's configured target directory, including CARGO_TARGET_DIR.
fn find_target_directory() -> Option<PathBuf> {
    cargo_target_directory(Path::new("cargo"))
}

fn cargo_target_directory(cargo: &Path) -> Option<PathBuf> {
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version=1"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    metadata
        .get("target_directory")?
        .as_str()
        .map(PathBuf::from)
}

/// Query a newline-delimited RPC response without waiting for connection closure.
fn query_rpc_status(host: &str, port: u16, timeout: Duration) -> Result<serde_json::Value> {
    let host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    let addresses = (host, port).to_socket_addrs().c(d!("resolve RPC host"))?;
    let mut stream = addresses
        .filter_map(|addr| TcpStream::connect_timeout(&addr, timeout).ok())
        .next()
        .ok_or_else(|| eg!("could not connect to RPC host {}:{}", host, port))?;
    stream
        .set_read_timeout(Some(timeout))
        .c(d!("set RPC read timeout"))?;
    stream
        .set_write_timeout(Some(timeout))
        .c(d!("set RPC write timeout"))?;
    stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"status\",\"params\":[]}\n")
        .c(d!("write RPC request"))?;
    read_rpc_result(BufReader::new(stream))
}

fn read_rpc_result(reader: impl BufRead) -> Result<serde_json::Value> {
    const MAX_RESPONSE_BYTES: u64 = 1_048_576;
    let mut response = Vec::new();
    reader
        .take(MAX_RESPONSE_BYTES + 1)
        .read_until(b'\n', &mut response)
        .c(d!("read RPC response"))?;
    if response.len() as u64 > MAX_RESPONSE_BYTES || response.last() != Some(&b'\n') {
        return Err(eg!("oversized or unterminated RPC response"));
    }
    let response: serde_json::Value =
        serde_json::from_slice(&response).c(d!("parse RPC response"))?;
    response
        .get("result")
        .filter(|result| result.is_object())
        .cloned()
        .ok_or_else(|| eg!("RPC response did not contain a status result"))
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
    use std::time::Instant;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if query_rpc_status(host, port, remaining.min(Duration::from_secs(1))).is_ok() {
            return true;
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

pub(crate) fn process_command_line(pid: u32) -> Option<String> {
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
    if !(name.contains("cluster-node") || name.contains("hotmint")) {
        return false;
    }

    let Some(command) = process_command_line(pid) else {
        return false;
    };
    command_targets_home(&command, home_dir)
}

/// Returns true if `command` passes `--home <home_dir>` (or `--home=<home_dir>`)
/// as an exact argument token. Matching the path token rather than a substring
/// prevents `/base/v1` from matching a process running with `--home /base/v10`.
pub(crate) fn command_targets_home(command: &str, home_dir: &Path) -> bool {
    let mut tokens = command.split_whitespace();
    while let Some(tok) = tokens.next() {
        let candidate = if tok == "--home" {
            tokens.next()
        } else {
            tok.strip_prefix("--home=")
        };
        if let Some(path) = candidate
            && paths_equal(Path::new(path), home_dir)
        {
            return true;
        }
    }
    false
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::thread;

    pub(crate) struct TestDir(pub PathBuf);

    impl TestDir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!("hotmint-mgmt-{}", rand::random::<u64>()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn check_rpc_host(host: &str, bind_host: &str) {
        let listener = TcpListener::bind((bind_host, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, wait) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert!(request.contains("status"));
            stream.write_all(b"{\"result\": {\"last_committed_height\": 7, \"current_view\": 8, \"epoch\": 2}}\n").unwrap();
            // Keep the socket open until the client finishes; EOF is not the frame boundary.
            let _ = wait.recv_timeout(Duration::from_secs(3));
        });
        let result = query_rpc_status(host, port, Duration::from_secs(1));
        release.send(()).unwrap();
        server.join().unwrap();
        assert_eq!(result.unwrap()["last_committed_height"], 7);
    }

    #[test]
    fn rpc_status_supports_dns_and_persistent_connections() {
        check_rpc_host("localhost", "127.0.0.1");
    }

    #[test]
    fn rpc_status_supports_ipv6() {
        if TcpListener::bind(("::1", 0)).is_ok() {
            check_rpc_host("::1", "::1");
            check_rpc_host("[::1]", "::1");
        }
        assert_eq!(format_host_port("[::1]", 80), "[::1]:80");
    }

    #[test]
    fn rpc_status_rejects_oversized_truncated_and_error_responses() {
        assert!(read_rpc_result(&b"{\"result\":{}}"[..]).is_err());
        assert!(read_rpc_result(&b"{\"error\":{\"code\":-1}}\n"[..]).is_err());
        let oversized = vec![b' '; 1_048_577];
        assert!(read_rpc_result(oversized.as_slice()).is_err());
    }

    #[test]
    fn build_drains_output_and_uses_cargo_target_directory() {
        let temp = TestDir::new();
        let target = temp.0.join("target with a \\\" quote");
        fs::create_dir_all(target.join("release")).unwrap();
        let expected = target.join("release/test-node");
        fs::write(&expected, []).unwrap();
        let metadata = serde_json::json!({"target_directory": target});
        fs::write(
            temp.0.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        let cargo = temp.0.join("cargo");
        fs::write(&cargo, b"#!/bin/sh\nif [ \"$1\" = build ]; then\n  dd if=/dev/zero bs=1048576 count=1 2>/dev/null\n  dd if=/dev/zero bs=1048576 count=1 1>&2 2>/dev/null\nelse\n  cat \"$(dirname \"$0\")/metadata.json\"\nfi\n").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let (send, recv) = mpsc::channel();
        let build = thread::spawn(move || {
            send.send(build_binary_with_cargo(&cargo, "test-node", None))
                .unwrap();
        });
        let result = recv
            .recv_timeout(Duration::from_secs(5))
            .expect("build output must be drained");
        build.join().unwrap();
        assert_eq!(result, Some(expected));
    }
}
