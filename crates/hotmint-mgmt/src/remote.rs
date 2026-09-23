//! Remote (distributed) deployment via SSH + git-based sync.

use ruc::*;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process;

use crate::cluster::ClusterState;
use serde::{Deserialize, Serialize};

/// Shell-escape a string by wrapping it in single quotes, with proper handling
/// of embedded single quotes.  e.g. `foo'bar` -> `'foo'\''bar'`
///
/// Leading `~/` is replaced with `$HOME/` so that tilde expansion works
/// even inside single quotes (the `$HOME` is spliced outside the quotes).
fn shell_escape(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        // $HOME must be outside quotes for expansion
        format!("\"$HOME\"/'{}'", rest.replace('\'', "'\\''"))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn remote_home_for_host(host: &HostEntry) -> String {
    host.home
        .clone()
        .unwrap_or_else(|| format!("~/hotmint-v{}", host.validator_id))
}

fn remote_child_path(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

fn remote_pid_file(remote_home: &str) -> String {
    remote_child_path(remote_home, "hotmint.pid")
}

fn remote_log_file(remote_home: &str) -> String {
    remote_child_path(remote_home, "hotmint.log")
}

/// Remote host configuration (from hosts.toml).
#[derive(Debug, Serialize, Deserialize)]
pub struct HostsConfig {
    pub hosts: Vec<HostEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HostEntry {
    /// Validator ID assigned to this host.
    pub validator_id: u64,
    /// SSH address: user@hostname or user@ip.
    pub ssh: String,
    /// External IP for P2P (if different from SSH host).
    pub external_ip: Option<String>,
    /// Remote home directory for the node.
    pub home: Option<String>,
}

impl HostsConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).c(d!("read hosts.toml"))?;
        toml::from_str(&contents).c(d!("parse hosts.toml"))
    }
}

/// Run an SSH command and return stdout.
fn ssh_exec(target: &str, cmd: &str) -> Result<String> {
    let output = process::Command::new("ssh")
        .args(["-o", "BatchMode=yes"])
        .arg(target)
        .arg(cmd)
        .output()
        .c(d!("ssh to {}", target))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eg!("ssh {} failed: {}", target, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Ensure remote has the repo at the correct commit.
fn git_sync(
    target: &str,
    repo_url: &str,
    branch: &str,
    expected_commit: &str,
    remote_dir: &str,
) -> Result<()> {
    // Check if repo exists
    let has_repo = ssh_exec(
        target,
        &format!(
            "test -d {}/.git && echo yes || echo no",
            shell_escape(remote_dir)
        ),
    )?
    .trim()
    .to_string();

    if has_repo == "no" {
        // First time: clone
        println!("    Cloning repository...");
        ssh_exec(
            target,
            &format!(
                "git clone {} {}",
                shell_escape(repo_url),
                shell_escape(remote_dir)
            ),
        )?;
    } else {
        // Fetch latest
        ssh_exec(
            target,
            &format!("cd {} && git fetch origin", shell_escape(remote_dir)),
        )?;
    }

    // Checkout and reset to exact commit
    ssh_exec(
        target,
        &format!(
            "cd {} && git checkout {} && git reset --hard origin/{}",
            shell_escape(remote_dir),
            shell_escape(branch),
            shell_escape(branch)
        ),
    )?;

    // Verify commit matches
    let remote_commit = ssh_exec(
        target,
        &format!("cd {} && git rev-parse HEAD", shell_escape(remote_dir)),
    )?
    .trim()
    .to_string();

    if remote_commit != expected_commit {
        return Err(eg!(
            "commit mismatch on {}: local={} remote={}",
            target,
            expected_commit,
            remote_commit
        ));
    }

    Ok(())
}

/// Write file content to a remote path by piping through ssh stdin.
fn ssh_write_file(target: &str, remote_path: &str, content: &[u8]) -> Result<()> {
    let mut command = process::Command::new("ssh");
    command
        .args(["-o", "BatchMode=yes", "--"])
        .arg(target)
        .arg(write_file_command(remote_path));
    write_command_input(&mut command, content)
}

fn write_file_command(remote_path: &str) -> String {
    let path = shell_escape(remote_path);
    format!("umask 077; if [ -e {path} ]; then chmod 600 -- {path} || exit 1; fi; cat > {path}")
}

fn write_command_input(command: &mut process::Command, content: &[u8]) -> Result<()> {
    let mut child = command
        .stdin(process::Stdio::piped())
        .spawn()
        .c(d!("spawn file transfer"))?;
    let write_result = child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(content);
    // Close stdin and reap the child even if it rejects the input early.
    let status = child.wait().c(d!("wait file transfer"))?;
    write_result.c(d!("write file content"))?;
    if !status.success() {
        return Err(eg!("file transfer failed: {}", status));
    }
    Ok(())
}

fn pipe_commands(producer: &mut process::Command, consumer: &mut process::Command) -> Result<()> {
    let mut producer = producer
        .stdout(process::Stdio::piped())
        .spawn()
        .c(d!("spawn archive producer"))?;
    let output = producer.stdout.take().expect("stdout was piped");
    let spawned = consumer.stdin(output).spawn();
    // Drop the parent's read end so an early consumer exit signals the producer.
    consumer.stdin(process::Stdio::null());
    let mut consumer = match spawned {
        Ok(child) => child,
        Err(err) => {
            let _ = producer.kill();
            let _ = producer.wait();
            return Err(eg!("spawn archive consumer: {}", err));
        }
    };
    // Both ends run concurrently; reap both before propagating either error.
    let producer_status = producer.wait();
    let consumer_status = consumer.wait();
    let producer_status = producer_status.c(d!("wait archive producer"))?;
    let consumer_status = consumer_status.c(d!("wait archive consumer"))?;
    if !producer_status.success() || !consumer_status.success() {
        return Err(eg!(
            "archive transfer failed: producer {}, consumer {}",
            producer_status,
            consumer_status
        ));
    }
    Ok(())
}

/// Get the current local HEAD commit hash.
fn get_local_commit() -> Result<String> {
    let output = process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .c(d!("get local git commit"))?;
    if !output.status.success() {
        return Err(eg!("failed to get local git commit"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn deploy(
    base_dir: &Path,
    hosts_path: &Path,
    package: &str,
    repo_url: &str,
    branch: &str,
) -> Result<()> {
    let state = ClusterState::load(base_dir)?;
    let hosts = HostsConfig::load(hosts_path)?;

    // Get local commit hash for verification
    let local_commit = get_local_commit()?;

    println!(
        "Deploying {} (branch: {}, commit: {})",
        state.chain_id,
        branch,
        &local_commit[..8]
    );

    for host in &hosts.hosts {
        let vid = host.validator_id;
        let remote_home = remote_home_for_host(host);
        let remote_src = "~/hotmint";

        // Validate
        let _v = state
            .validators
            .iter()
            .find(|v| v.id == vid)
            .ok_or_else(|| eg!("validator {} not found in cluster state", vid))?;

        println!("\n--- V{} ({}) ---", vid, host.ssh);

        // 1. Git sync
        println!("  Syncing via git (branch: {})...", branch);
        git_sync(&host.ssh, repo_url, branch, &local_commit, remote_src)?;

        // 2. Config files via ssh pipe
        let local_config = base_dir.join(format!("v{}", vid)).join("config");
        if local_config.exists() {
            println!("  Syncing config...");
            ssh_exec(
                &host.ssh,
                &format!("mkdir -p {}/config", shell_escape(&remote_home)),
            )?;
            for file in [
                "config.toml",
                "genesis.json",
                "priv_validator_key.json",
                "node_key.json",
            ] {
                let local_file = local_config.join(file);
                if local_file.exists() {
                    let content = fs::read(&local_file).c(d!("read config file"))?;
                    let remote_path = format!("{}/config/{}", remote_home, file);
                    ssh_write_file(&host.ssh, &remote_path, &content)?;
                    // Restrict permissions on key files
                    if file.contains("key") {
                        ssh_exec(
                            &host.ssh,
                            &format!("chmod 600 {}", shell_escape(&remote_path)),
                        )?;
                    }
                }
            }
        }

        // 3. Build on remote (use set -o pipefail so build failure propagates through pipe)
        println!("  Building {} on {}...", package, host.ssh);
        let build_output = ssh_exec(
            &host.ssh,
            &format!(
                "set -o pipefail; cd {} && cargo build --release -p {} 2>&1 | tail -5",
                shell_escape(remote_src),
                shell_escape(package),
            ),
        )?;
        println!("  {}", build_output.trim());

        // 4. Stop any existing node, then start
        println!("  Starting V{}...", vid);
        let bin_path = format!("{}/target/release/{}", remote_src, package);
        let pid_file = remote_pid_file(&remote_home);
        let log_file = remote_log_file(&remote_home);
        let esc_pid = shell_escape(&pid_file);
        let esc_bin = shell_escape(&bin_path);
        let esc_home = shell_escape(&remote_home);
        let esc_log = shell_escape(&log_file);
        ssh_exec(
            &host.ssh,
            &format!(
                "mkdir -p {esc_home}; \
                 if [ -f {esc_pid} ]; then \
                   pid=$(cat {esc_pid} 2>/dev/null || true); \
                   case \"$pid\" in ''|*[!0-9]*) ;; *) \
                     if ps -p \"$pid\" -o command= | grep -F -- {esc_bin} >/dev/null && \
                        ps -p \"$pid\" -o command= | grep -F -- '--home' >/dev/null && \
                        ps -p \"$pid\" -o command= | grep -F -- {esc_home} >/dev/null; then \
                       kill \"$pid\" 2>/dev/null; sleep 1; \
                     fi ;; \
                   esac; \
                   rm -f {esc_pid}; \
                 fi; \
                 nohup {esc_bin} --home {esc_home} > {esc_log} 2>&1 & echo $! > {esc_pid}",
            ),
        )?;
        println!("  V{}: started on {}", vid, host.ssh);
    }

    println!("\nDeployment complete.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Cluster-wide operations (chaindev-style)
// ---------------------------------------------------------------------------

/// Execute a command on all remote hosts and print results.
pub fn exec_all(hosts_path: &Path, cmd: &str) -> Result<()> {
    let hosts = HostsConfig::load(hosts_path)?;
    for host in &hosts.hosts {
        println!("--- V{} ({}) ---", host.validator_id, host.ssh);
        match ssh_exec(&host.ssh, cmd) {
            Ok(output) => print!("{}", output),
            Err(e) => eprintln!("  ERROR: {e}"),
        }
    }
    Ok(())
}

/// Push a local file or directory to all remote hosts.
pub fn push_all(hosts_path: &Path, local: &Path, remote_dest: &str) -> Result<()> {
    let hosts = HostsConfig::load(hosts_path)?;
    for host in &hosts.hosts {
        println!("--- V{} ({}) ---", host.validator_id, host.ssh);
        if local.is_dir() {
            // Pass local paths and SSH destinations as arguments, never shell code.
            let mut archive = process::Command::new("tar");
            archive.args(["czf", "-", "-C"]).arg(local).arg(".");
            let dest = shell_escape(remote_dest);
            let mut transfer = process::Command::new("ssh");
            transfer
                .args(["-o", "BatchMode=yes", "--"])
                .arg(&host.ssh)
                .arg(format!("mkdir -p -- {dest} && cd {dest} && tar xzf -"));
            pipe_commands(&mut archive, &mut transfer)?;
        } else {
            // Single file: read and pipe through ssh
            let content = fs::read(local).c(d!("read local file"))?;
            ssh_write_file(&host.ssh, remote_dest, &content)?;
        }
        println!("  OK");
    }
    Ok(())
}

/// Pull a file from all remote hosts into a local directory.
/// Each file is saved as `{local_dir}/V{id}_{filename}`.
pub fn pull_all(hosts_path: &Path, remote_src: &str, local_dir: &Path) -> Result<()> {
    let hosts = HostsConfig::load(hosts_path)?;
    fs::create_dir_all(local_dir).c(d!("create local dir"))?;

    let filename = Path::new(remote_src)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());

    for host in &hosts.hosts {
        let dest = local_dir.join(format!("V{}_{}", host.validator_id, filename));
        println!(
            "--- V{} ({}) → {} ---",
            host.validator_id,
            host.ssh,
            dest.display()
        );
        let status = process::Command::new("scp")
            .args(["-o", "BatchMode=yes"])
            .arg(format!("{}:{}", host.ssh, remote_src))
            .arg(&dest)
            .status()
            .c(d!("scp from {}", host.ssh))?;
        if !status.success() {
            eprintln!("  ERROR: pull from {} failed", host.ssh);
        } else {
            println!("  OK");
        }
    }
    Ok(())
}

/// Collect, tail, or grep logs from remote nodes.
pub fn logs(
    hosts_path: &Path,
    lines: u32,
    grep: Option<&str>,
    collect_dir: Option<&Path>,
) -> Result<()> {
    let hosts = HostsConfig::load(hosts_path)?;

    if let Some(dir) = collect_dir {
        // Download all log files
        fs::create_dir_all(dir).c(d!("create collect dir"))?;
        for host in &hosts.hosts {
            let vid = host.validator_id;
            let remote_home = remote_home_for_host(host);
            let remote_log = remote_log_file(&remote_home);
            let local_path = dir.join(format!("V{}.log", vid));
            println!(
                "Collecting V{} ({}) → {}",
                vid,
                host.ssh,
                local_path.display()
            );
            let status = process::Command::new("scp")
                .args(["-o", "BatchMode=yes"])
                .arg(format!("{}:{}", host.ssh, remote_log))
                .arg(&local_path)
                .status()
                .c(d!("scp log from {}", host.ssh))?;
            if !status.success() {
                eprintln!("  WARNING: failed to collect log from {}", host.ssh);
            }
        }
        println!("Logs collected to {}", dir.display());
        return Ok(());
    }

    // Tail + optional grep
    for host in &hosts.hosts {
        let vid = host.validator_id;
        let remote_home = remote_home_for_host(host);
        let remote_log = remote_log_file(&remote_home);
        let cmd = if let Some(pattern) = grep {
            format!(
                "tail -n {} {} | grep --color=never {}",
                lines,
                shell_escape(&remote_log),
                shell_escape(pattern),
            )
        } else {
            format!("tail -n {} {}", lines, shell_escape(&remote_log))
        };

        println!("=== V{} ({}) ===", vid, host.ssh);
        match ssh_exec(&host.ssh, &cmd) {
            Ok(output) => print!("{}", output),
            Err(e) => eprintln!("  ERROR: {e}"),
        }
    }
    Ok(())
}

/// Show status of all remote nodes (process alive, RPC height/view/epoch).
pub fn remote_status(base_dir: &Path, hosts_path: &Path) -> Result<()> {
    let state = ClusterState::load(base_dir)?;
    let hosts = HostsConfig::load(hosts_path)?;

    println!(
        "{:<6} {:<20} {:<8} {:<10} {:<8} {:<8}",
        "NODE", "HOST", "PID", "HEIGHT", "VIEW", "EPOCH"
    );
    println!("{}", "-".repeat(62));

    for host in &hosts.hosts {
        let vid = host.validator_id;
        let remote_home = remote_home_for_host(host);
        let pid_file = remote_pid_file(&remote_home);
        let esc_pid = shell_escape(&pid_file);
        let esc_home = shell_escape(&remote_home);

        // Check if process is alive
        let pid_info = match ssh_exec(
            &host.ssh,
            &format!(
                "if [ -f {esc_pid} ]; then \
                   pid=$(cat {esc_pid} 2>/dev/null || true); \
                   case \"$pid\" in ''|*[!0-9]*) echo stale ;; *) \
                     if ps -p \"$pid\" -o command= | grep -F -- '--home' >/dev/null && \
                        ps -p \"$pid\" -o command= | grep -F -- {esc_home} >/dev/null; then \
                       echo \"alive:$pid\"; \
                     else \
                       echo stale; \
                     fi ;; \
                   esac; \
                 else echo none; fi"
            ),
        ) {
            Ok(s) => s.trim().to_string(),
            Err(_) => "ssh-err".into(),
        };

        let (status_str, pid_str) = if pid_info.starts_with("alive:") {
            ("UP", pid_info.strip_prefix("alive:").unwrap_or("?"))
        } else if pid_info == "dead" {
            ("DOWN", "-")
        } else if pid_info == "none" {
            ("NONE", "-")
        } else if pid_info == "stale" {
            ("STALE", "-")
        } else {
            ("ERR", "-")
        };

        // Try RPC status if node is up and we know its port
        let (height, view, epoch) = if status_str == "UP" {
            if let Some(v) = state.validators.iter().find(|v| v.id == vid) {
                let rpc_host = host
                    .external_ip
                    .as_deref()
                    .unwrap_or_else(|| host.ssh.split('@').next_back().unwrap_or(&host.ssh));
                match query_rpc_status(rpc_host, v.rpc_port) {
                    Ok((h, v, e)) => (h, v, e),
                    Err(_) => ("-".into(), "-".into(), "-".into()),
                }
            } else {
                ("-".into(), "-".into(), "-".into())
            }
        } else {
            ("-".into(), "-".into(), "-".into())
        };

        println!(
            "{:<6} {:<20} {:<8} {:<10} {:<8} {:<8}",
            format!("V{}", vid),
            host.ssh,
            if status_str == "UP" {
                pid_str
            } else {
                status_str
            },
            height,
            view,
            epoch,
        );
    }
    Ok(())
}

/// Query a node's JSON-RPC status endpoint.
fn query_rpc_status(host: &str, port: u16) -> Result<(String, String, String)> {
    use std::time::Duration;

    let result = crate::query_rpc_status(host, port, Duration::from_secs(2))?;
    let extract = |key: &str| -> String {
        result
            .get(key)
            .and_then(|value| value.as_u64())
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".into())
    };
    Ok((
        extract("last_committed_height"),
        extract("current_view"),
        extract("epoch"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TestDir;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn directory_pipeline_handles_literal_paths_and_copies_contents() {
        let temp = TestDir::new();
        let source = temp.0.join("source ' ; $() directory");
        let dest = temp.0.join("destination ' ; $() directory");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("payload"), [0, 255, 1, 128]).unwrap();
        let mut archive = process::Command::new("tar");
        archive.args(["czf", "-", "-C"]).arg(&source).arg(".");
        let escaped = shell_escape(dest.to_str().unwrap());
        let mut extract = process::Command::new("sh");
        extract.arg("-c").arg(format!(
            "mkdir -p -- {escaped} && cd {escaped} && tar xzf -"
        ));
        pipe_commands(&mut archive, &mut extract).unwrap();
        assert_eq!(fs::read(dest.join("payload")).unwrap(), [0, 255, 1, 128]);
    }

    #[test]
    fn pipeline_propagates_producer_and_consumer_failures() {
        let mut fail = process::Command::new("sh");
        fail.args(["-c", "exit 7"]);
        let mut drain = process::Command::new("cat");
        drain.stdout(process::Stdio::null());
        assert!(pipe_commands(&mut fail, &mut drain).is_err());

        let mut producer = process::Command::new("dd");
        producer
            .args(["if=/dev/zero", "bs=1048576", "count=1"])
            .stderr(process::Stdio::null());
        let mut reject = process::Command::new("sh");
        reject.args(["-c", "exit 9"]);
        assert!(pipe_commands(&mut producer, &mut reject).is_err());
    }

    #[test]
    fn binary_file_transfer_restricts_new_and_existing_files() {
        let temp = TestDir::new();
        let dest = temp.0.join("file ' ; $() key");
        let content = [0, 128, 255, 2];
        for existing in [false, true] {
            if existing {
                fs::set_permissions(&dest, fs::Permissions::from_mode(0o644)).unwrap();
            }
            let mut command = process::Command::new("sh");
            command
                .arg("-c")
                .arg(write_file_command(dest.to_str().unwrap()));
            write_command_input(&mut command, &content).unwrap();
            assert_eq!(fs::read(&dest).unwrap(), content);
            assert_eq!(
                fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
