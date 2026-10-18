//! Task 030 — end-to-end acceptance tests.
//!
//! Unlike the focused crate tests, these drive the **real
//! `service-orchestrator` binary**: a real daemon over the real Unix-domain
//! socket IPC, a real MCP stdio server, real local Git repositories with a real
//! bare remote, a real Node.js HTTP fixture service, and real health probes.
//!
//! Everything runs in a throwaway temporary directory with a private socket.
//! No real repository, database, credential, or developer service is ever
//! touched. Each scenario owns its own daemon and socket; a process-wide lock
//! serialises the scenarios because the socket path is carried in a
//! process-global environment variable.
//!
//! These tests are intentionally hermetic and fast. The heavier, toolchain- and
//! network-dependent Maven/Liquibase acceptance runs are the `#[ignore]`d crate
//! tests, executed separately (see `docs/acceptance-test-matrix.md`).

#![cfg(unix)]

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use service_orchestrator_application::HealthCheckScope;
use service_orchestrator_client::{ClientKind, OrchestratorClient, RemoteClient, SOCKET_ENV};
use service_orchestrator_domain::{
    Branch, BranchKind, ProcessState, RepositoryLifecycle, ServiceId, Task, TaskId, TaskStatus,
    WorkspaceId,
};

/// How long any single acceptance interaction may take.
const DEADLINE: Duration = Duration::from_secs(45);

/// Serialises the scenarios so the process-global socket override cannot race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Fixture construction
// ---------------------------------------------------------------------------

/// Runs `git` in `dir` with a deterministic, isolated environment.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Acceptance")
        .env("GIT_AUTHOR_EMAIL", "acceptance@example.com")
        .env("GIT_COMMITTER_NAME", "Acceptance")
        .env("GIT_COMMITTER_EMAIL", "acceptance@example.com")
        .env("LC_ALL", "C")
        .output()
        .expect("git is installed and runnable");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository created as a bare remote, a seed clone, and a working clone.
struct Repo {
    /// A working clone used to push new commits to the remote.
    seed: PathBuf,
    /// The default branch of the remote.
    branch: String,
}

/// Creates a bare remote, a seed clone, and a working checkout for `name`.
fn create_repo(root: &Path, workspace_root: &Path, name: &str, rel: &str, branch: &str) -> Repo {
    let remote = root.join("remotes").join(format!("{name}.git"));
    let seed = root.join("seeds").join(name);
    let checkout = workspace_root.join(rel);
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-b", branch]);
    std::fs::create_dir_all(&seed).unwrap();
    git(&seed, &["init", "-b", branch]);
    std::fs::write(seed.join("README.md"), format!("# {name}\n")).unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "initial"]);
    git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&seed, &["push", "-u", "origin", branch]);
    std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
    git(
        workspace_root,
        &[
            "clone",
            remote.to_str().unwrap(),
            checkout.to_str().unwrap(),
        ],
    );
    Repo {
        seed,
        branch: branch.to_owned(),
    }
}

/// Pushes a new commit to a repository's remote default branch.
fn remote_commit(repo: &Repo, name: &str) {
    std::fs::write(repo.seed.join(format!("{name}.txt")), "content\n").unwrap();
    git(&repo.seed, &["add", "."]);
    git(&repo.seed, &["commit", "-m", name]);
    git(&repo.seed, &["push", "origin", &repo.branch]);
}

/// Creates and pushes a new branch on a repository's remote.
fn remote_branch(repo: &Repo, name: &str) {
    git(&repo.seed, &["checkout", "-b", name]);
    std::fs::write(repo.seed.join(format!("{name}.txt")), "branch\n").unwrap();
    git(&repo.seed, &["add", "."]);
    git(&repo.seed, &["commit", "-m", name]);
    git(&repo.seed, &["push", "-u", "origin", name]);
    git(&repo.seed, &["checkout", &repo.branch]);
}

/// A free loopback TCP port.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port is available");
    listener.local_addr().unwrap().port()
}

/// A minimal Node HTTP service that reports `{"status":"UP"}` on its port.
const SERVE_JS: &str = "\
const http = require('http');
const port = Number(process.env.PORT || 0);
const server = http.createServer((_req, res) => {
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ status: 'UP' }));
});
server.listen(port, '127.0.0.1', () => console.log('listening on ' + port));
";

// ---------------------------------------------------------------------------
// The isolated environment
// ---------------------------------------------------------------------------

/// A running, isolated daemon plus the throwaway world it owns.
struct Env {
    /// Held for the environment lifetime so only one scenario mutates the
    /// process-global socket variable.
    lock: MutexGuard<'static, ()>,
    previous_socket: Option<OsString>,
    root: PathBuf,
    workspace_root: PathBuf,
    config_path: PathBuf,
    socket: PathBuf,
    daemon: Child,
    daemon_log: PathBuf,
    billing: Repo,
}

impl Env {
    /// Builds the fixture world and starts a real daemon against it.
    fn start() -> Self {
        Self::launch(false)
    }

    /// Starts a daemon over a directory that already holds a stale socket file
    /// and a stale lock file, to prove both are recovered safely.
    fn start_over_stale_runtime() -> Self {
        Self::launch(true)
    }

    fn launch(stale: bool) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let root = std::env::temp_dir().join(format!(
            "so-accept-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the clock is after the epoch")
                .as_nanos()
        ));
        let workspace_root = root.join("workspace");
        std::fs::create_dir_all(&workspace_root).unwrap();
        std::fs::create_dir_all(root.join("seeds")).unwrap();
        std::fs::create_dir_all(root.join("remotes")).unwrap();

        let _orders = create_repo(
            &root,
            &workspace_root,
            "orders-api",
            "backend/orders-api",
            "main",
        );
        let billing = create_repo(
            &root,
            &workspace_root,
            "billing-api",
            "backend/billing-api",
            "main",
        );
        create_repo(
            &root,
            &workspace_root,
            "slow-api",
            "backend/slow-api",
            "main",
        );
        create_repo(
            &root,
            &workspace_root,
            "slow-api-2",
            "backend/slow-api-2",
            "main",
        );
        create_repo(
            &root,
            &workspace_root,
            "shared-kit",
            "libraries/shared-kit",
            "main",
        );
        // `orders-api` is intentionally left dirty: its untracked `serve.js` is
        // the local change the bulk-sync scenario must preserve.
        std::fs::write(workspace_root.join("backend/orders-api/serve.js"), SERVE_JS).unwrap();

        let orders_port = free_port();
        let orders_remote = root.join("remotes/orders-api.git");
        let billing_remote = root.join("remotes/billing-api.git");
        let slow_remote = root.join("remotes/slow-api.git");
        let slow2_remote = root.join("remotes/slow-api-2.git");
        let shared_remote = root.join("remotes/shared-kit.git");

        let config = format!(
            r#"config_version = 1

[[workspaces]]
id = "accept"
name = "Acceptance"
root = "{workspace_root}"

[[libraries]]
id = "shared-kit"
name = "shared-kit"
workspace = "accept"
path = "libraries/shared-kit"
type = "maven"
build_system = "maven"

[libraries.repository]
url = "{shared_remote}"
default_branch = "main"

[libraries.commands]
build = {{ program = "node", args = ["--version"] }}

[libraries.artifact]
group_id = "com.accept"
artifact_id = "shared-kit"
version = "1.0.0"

[[services]]
id = "orders-api"
name = "orders-api"
type = "backend"
workspace = "accept"
path = "backend/orders-api"
default_branch = "main"
build_system = "npm"

[services.repository]
url = "{orders_remote}"

[services.runtime]
type = "node"
version = "20.x"

[services.commands]
build = {{ program = "node", args = ["--version"] }}
start = {{ program = "node", args = ["serve.js"] }}

[services.environment]
PORT = "{orders_port}"

[services.dependencies]
libraries = ["shared-kit"]

[[services.ports]]
port = {orders_port}
protocol = "tcp"

[[services.health_checks]]
type = "http"
url = "http://127.0.0.1:{orders_port}/health"
timeout_seconds = 5

[[services]]
id = "billing-api"
name = "billing-api"
type = "backend"
workspace = "accept"
path = "backend/billing-api"
default_branch = "main"
build_system = "npm"

[services.repository]
url = "{billing_remote}"

[services.runtime]
type = "node"
version = "20.x"

[services.commands]
build = {{ program = "node", args = ["--version"] }}
start = {{ program = "node", args = ["serve.js"] }}

[services.dependencies]
services = ["orders-api"]

[[services]]
id = "slow-api"
name = "slow-api"
type = "backend"
workspace = "accept"
path = "backend/slow-api"
default_branch = "main"
build_system = "npm"

[services.repository]
url = "{slow_remote}"

[services.runtime]
type = "node"
version = "20.x"

[services.commands]
build = {{ program = "sleep", args = ["30"] }}
start = {{ program = "sleep", args = ["60"] }}

[[services]]
id = "slow-api-2"
name = "slow-api-2"
type = "backend"
workspace = "accept"
path = "backend/slow-api-2"
default_branch = "main"
build_system = "npm"

[services.repository]
url = "{slow2_remote}"

[services.runtime]
type = "node"
version = "20.x"

[services.commands]
build = {{ program = "sleep", args = ["30"] }}
start = {{ program = "sleep", args = ["60"] }}

[[services]]
id = "docs-site"
name = "docs-site"
type = "frontend"
workspace = "accept"
path = "frontend/docs-site"
default_branch = "main"
build_system = "npm"

[services.repository]
url = "https://example.invalid/docs-site.git"

[services.runtime]
type = "node"
version = "20.x"

[services.commands]
build = {{ program = "node", args = ["--version"] }}
start = {{ program = "node", args = ["serve.js"] }}

[[groups]]
id = "core"
name = "Core"
services = ["orders-api", "billing-api"]

[[profiles]]
id = "backend"
name = "Backend"
workspace = "accept"
services = ["orders-api"]

[execution]
max_parallel_tasks = 1
default_timeout_seconds = 60
"#,
            workspace_root = workspace_root.display(),
            shared_remote = shared_remote.display(),
            orders_remote = orders_remote.display(),
            billing_remote = billing_remote.display(),
            slow_remote = slow_remote.display(),
            slow2_remote = slow2_remote.display(),
            orders_port = orders_port,
        );
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, config).unwrap();

        let socket = root.join("run/daemon.sock");
        let previous_socket = std::env::var_os(SOCKET_ENV);
        std::env::set_var(SOCKET_ENV, &socket);
        assert_eq!(
            service_orchestrator_client::socket_path(&config_path),
            socket,
            "the environment pins the socket path exactly"
        );

        if stale {
            std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
            std::fs::write(&socket, b"stale").unwrap();
            std::fs::write(socket.with_extension("lock"), b"stale").unwrap();
        }

        let daemon_log = root.join("daemon.log");
        let log = std::fs::File::create(&daemon_log).unwrap();
        let log_err = log.try_clone().unwrap();
        let exe = env!("CARGO_BIN_EXE_service-orchestrator");
        let daemon = Command::new(exe)
            .arg("daemon")
            .arg("--config")
            .arg(&config_path)
            .env(SOCKET_ENV, &socket)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .expect("the daemon binary launches");

        let env = Self {
            lock,
            previous_socket,
            root,
            workspace_root,
            config_path,
            socket,
            daemon,
            daemon_log,
            billing,
        };
        env.wait_ready();
        env
    }

    /// Connects a fresh client, never starting a daemon of its own.
    fn client(&self, kind: ClientKind) -> RemoteClient {
        RemoteClient::connect_existing(&self.config_path, kind).expect("the daemon is reachable")
    }

    /// Reads the daemon's captured output (for diagnostics in the matrix).
    fn daemon_output(&self) -> String {
        std::fs::read_to_string(&self.daemon_log).unwrap_or_default()
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if UnixStream::connect(&self.socket).is_ok() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the daemon did not become reachable in time; log:\n{}",
                self.daemon_output()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // Keep the lock held until the environment is restored.
        let _guard = &self.lock;
        if let Ok(client) = RemoteClient::connect_existing(&self.config_path, ClientKind::Cli) {
            let _ = client.shutdown_daemon();
        }
        wait_for_exit(&mut self.daemon, "daemon");
        match &self.previous_socket {
            Some(value) => std::env::set_var(SOCKET_ENV, value),
            None => std::env::remove_var(SOCKET_ENV),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Waits for a child to exit, returning its exit code.
fn wait_for_exit(child: &mut Child, label: &str) -> Option<i32> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match child.try_wait().expect("the child is waitable") {
            Some(status) => return status.code(),
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the {label} did not exit in time");
            }
        }
    }
}

/// Polls `probe` until it yields a value, or fails at the deadline.
fn wait_for<T>(label: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits until a task reaches a terminal state and returns it. Cancels a task
/// that overruns the deadline so the harness never leaks work.
fn wait_for_terminal(client: &RemoteClient, task_id: &TaskId) -> Task {
    wait_for(&format!("task {task_id} to settle"), || {
        let task = client.get_task(task_id).expect("the task is found");
        task.status.is_terminal().then_some(task)
    })
}

fn service(id: &str) -> ServiceId {
    ServiceId::new(id).expect("a valid service id")
}

fn workspace() -> WorkspaceId {
    WorkspaceId::new("accept").expect("a valid workspace id")
}

// ---------------------------------------------------------------------------
// A real MCP stdio client on the same daemon
// ---------------------------------------------------------------------------

/// A running `service-orchestrator mcp` child spoken to over stdio.
struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    next_id: u64,
}

impl McpClient {
    /// Launches the MCP server against the environment's daemon.
    fn launch(env: &Env) -> Self {
        let exe = env!("CARGO_BIN_EXE_service-orchestrator");
        let mut child = Command::new(exe)
            .arg("mcp")
            .arg("--config")
            .arg(&env.config_path)
            .env(SOCKET_ENV, &env.socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the mcp binary launches");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin: Some(stdin),
            stdout: lines,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let stdin = self.stdin.as_mut().expect("stdin is open");
        let mut line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .unwrap();
        line.push('\n');
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
        self.read_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        let mut line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .unwrap();
        line.push('\n');
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn read_response(&self, id: u64) -> Value {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("the server responded before the deadline");
            let line = self.stdout.recv_timeout(remaining).expect("a line arrives");
            let value: Value = serde_json::from_str(line.trim())
                .unwrap_or_else(|error| panic!("non-JSON on stdout: {line:?} ({error})"));
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return value;
            }
        }
    }

    /// Closes stdin, waits for a clean exit, and returns the exit code.
    fn shutdown(mut self) -> Option<i32> {
        drop(self.stdin.take());
        wait_for_exit(&mut self.child, "mcp server")
    }
}

/// Extracts the structured envelope from a `tools/call` result.
fn envelope(result: &Value) -> Value {
    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_null() {
            return structured.clone();
        }
    }
    let text = result["content"][0]["text"]
        .as_str()
        .expect("a tool result carries a content text block");
    serde_json::from_str(text).expect("the content text block is JSON")
}

// ===========================================================================
// Scenarios
// ===========================================================================

/// AC-01/AC-09 — the daemon boots, loads the workspace, lists entities, and
/// reports actionable workspace-validation diagnostics for a repository that
/// was never checked out.
#[test]
fn workspace_loads_and_validation_reports_actionable_diagnostics() {
    let env = Env::start();
    let client = env.client(ClientKind::Cli);

    let workspaces = client.list_workspaces().expect("workspaces are listed");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id.as_str(), "accept");

    let services = client
        .list_services(&workspace())
        .expect("services are listed");
    let ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "orders-api",
            "billing-api",
            "slow-api",
            "slow-api-2",
            "docs-site"
        ]
    );

    let report = client
        .validate_workspace(&workspace())
        .expect("the workspace validates");
    assert_eq!(report.workspace_id, workspace());
    assert_eq!(report.totals.services, 5);
    assert_eq!(report.totals.libraries, 1);
    // `docs-site` was configured but never cloned.
    assert_eq!(
        report.totals.repositories_missing, 1,
        "the missing docs-site checkout is counted: {:#?}",
        report.totals
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| format!("{:?}", d.code).contains("MissingRepository")),
        "a missing-repository diagnostic is produced: {:#?}",
        report.diagnostics
    );

    // An unknown workspace yields a structured, non-panicking error.
    let missing = client
        .validate_workspace(&WorkspaceId::new("nope").unwrap())
        .expect_err("an unknown workspace is an error");
    assert!(format!("{missing}").to_lowercase().contains("workspace"));
}

/// AC-10 — real Git: status, branches, fetch, remote branch discovery, pull,
/// and checkout against a local bare remote.
#[test]
fn git_status_fetch_pull_branches_and_checkout() {
    let env = Env::start();
    let client = env.client(ClientKind::Tui);
    let id = service("billing-api");

    let status = futures_block(client.get_git_status(&id)).expect("the git status is read");
    assert_eq!(status.lifecycle, RepositoryLifecycle::Available);
    assert_eq!(status.branch.as_ref().map(Branch::as_str), Some("main"));
    assert!(status.is_clean(), "the fresh clone is clean: {status:?}");

    let branches = futures_block(client.list_branches(&id)).expect("the branches are listed");
    assert!(
        branches.iter().any(|b| b.name == "main" && b.current),
        "the current local branch is listed: {branches:?}"
    );

    // A new branch and a new commit appear on the remote.
    remote_branch(&env.billing, "feature");
    remote_commit(&env.billing, "update");

    let fetched = client
        .fetch_repository(&id)
        .expect("the fetch is scheduled");
    let task_id = fetched.task_id().expect("a task id is returned").clone();
    let task = wait_for_terminal(&client, &task_id);
    assert_eq!(
        task.status,
        TaskStatus::Succeeded,
        "fetch succeeded: {task:?}"
    );

    let branches = futures_block(client.list_branches(&id)).expect("the branches are listed");
    assert!(
        branches
            .iter()
            .any(|b| b.name.contains("feature") && b.kind == BranchKind::Remote),
        "the fetched remote branch is visible: {branches:?}"
    );

    // The checkout is behind after the fetch; a pull fast-forwards it.
    let behind = futures_block(client.get_git_status(&id)).expect("the git status is read");
    assert!(
        behind.behind.unwrap_or(0) >= 1,
        "the checkout is behind its upstream after fetch: {behind:?}"
    );

    let pulled = client.pull_repository(&id).expect("the pull is scheduled");
    let pull_task = wait_for_terminal(&client, pulled.task_id().unwrap());
    assert_eq!(pull_task.status, TaskStatus::Succeeded);

    let after = futures_block(client.get_git_status(&id)).expect("the git status is read");
    assert!(after.is_clean(), "the fast-forward leaves a clean tree");
    assert_eq!(
        after.behind.unwrap_or(0),
        0,
        "the pull caught up with the upstream"
    );

    let checked_out = client
        .checkout_repository(&id, &Branch::new("feature").unwrap())
        .expect("the checkout is scheduled");
    let checkout_task = wait_for_terminal(&client, checked_out.task_id().unwrap());
    assert_eq!(checkout_task.status, TaskStatus::Succeeded);
    let on_feature = futures_block(client.get_git_status(&id)).expect("the git status is read");
    assert_eq!(
        on_feature.branch.as_ref().map(Branch::as_str),
        Some("feature"),
        "the repository is on the requested branch"
    );
}

/// AC-11 — bulk sync to the default branch: one dirty repository is refused
/// while the others continue; every result is reported and local changes are
/// preserved.
#[test]
fn bulk_sync_continues_when_one_repository_is_dirty() {
    let env = Env::start();
    let client = env.client(ClientKind::Tui);

    let scheduled = client
        .sync_repositories(
            &workspace(),
            &[service("orders-api"), service("billing-api")],
        )
        .expect("the bulk sync is scheduled");
    let workflow_id = scheduled
        .workflow_id()
        .expect("a workflow id is returned")
        .clone();

    let summary = wait_for("the bulk sync to settle", || {
        let summary = client
            .summarize_workflow(&workflow_id)
            .expect("the workflow is summarised");
        summary.is_complete().then_some(summary)
    });
    assert_eq!(summary.total, 2);
    assert_eq!(
        summary.succeeded, 1,
        "the clean repository succeeds independently: {summary:?}"
    );
    assert_eq!(
        summary.failed, 1,
        "the dirty repository is refused: {summary:?}"
    );

    // The dirty repository's local change is preserved untouched.
    assert!(
        env.workspace_root
            .join("backend/orders-api/serve.js")
            .is_file(),
        "the untracked local change survives a refused sync"
    );
    // The clean repository was actually synchronized.
    assert!(
        env.workspace_root
            .join("backend/billing-api/README.md")
            .is_file(),
        "the clean repository is still a valid checkout"
    );

    // The workflow status names the failure with a stable code.
    let status = client
        .get_workflow_status(&workflow_id)
        .expect("the workflow status is read");
    assert_eq!(status.failed, 1);
    assert!(
        !status.failures.is_empty(),
        "the failure carries a structured reason: {status:?}"
    );
}

/// AC-16/AC-17 — a real Node service starts, becomes healthy over HTTP, a
/// duplicate start is a no-op, and a stop transitions it back.
#[test]
fn service_start_health_duplicate_start_and_stop() {
    let env = Env::start();
    let client = env.client(ClientKind::Tui);
    let id = service("orders-api");

    let started = client.start_service(&id).expect("the start is scheduled");
    assert!(started.is_scheduled());

    let running = wait_for("orders-api to report running", || {
        let status = client
            .get_service_runtime_status(&id)
            .expect("the runtime status is read");
        (status.process_state == ProcessState::Running).then_some(status)
    });
    assert!(running.managed, "the daemon owns the process");
    assert!(running.pid.is_some(), "an OS pid is recorded");

    // A real HTTP health probe against the Node fixture reports healthy. The
    // process can report `running` slightly before its listener accepts, so the
    // probe is retried within the same bounded deadline instead of asserting on
    // the first attempt.
    let health = wait_for("the HTTP fixture to report healthy", || {
        let check = client
            .check_service_health(&id, HealthCheckScope::All)
            .ok()?;
        let task = wait_for_terminal(&client, check.task_id()?);
        if task.status != TaskStatus::Succeeded {
            return None;
        }
        let health = client.get_service_health(&id).ok()?;
        (health.state == service_orchestrator_application::HealthState::Healthy).then_some(health)
    });
    assert_eq!(
        health.state,
        service_orchestrator_application::HealthState::Healthy,
        "the HTTP fixture reports UP: {health:?}"
    );

    // A duplicate start is idempotent, never a second process.
    let again = client
        .start_service(&id)
        .expect("the duplicate start is handled");
    assert!(
        !again.is_scheduled(),
        "starting a running service is a no-op: {again:?}"
    );

    let stopped = client.stop_service(&id).expect("the stop is scheduled");
    assert!(stopped.is_scheduled());
    wait_for("orders-api to stop", || {
        let status = client
            .get_service_runtime_status(&id)
            .expect("the runtime status is read");
        (!status.process_state.is_active()).then_some(())
    });
}

/// AC-16/AC-28 — an unexpected service exit is observed and reported; the
/// daemon survives and never touches an unrelated process.
#[test]
fn unexpected_service_exit_is_reported() {
    let env = Env::start();
    let client = env.client(ClientKind::Tui);
    let id = service("orders-api");

    // An unrelated process the daemon must never signal.
    let mut decoy = Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("the decoy starts");

    client.start_service(&id).expect("the start is scheduled");
    let running = wait_for("orders-api to report running", || {
        let status = client
            .get_service_runtime_status(&id)
            .expect("the runtime status is read");
        (status.process_state == ProcessState::Running).then_some(status)
    });
    let pid = running.pid.expect("the service pid is known");

    // Kill the managed process out from under the daemon.
    let _ = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .expect("kill runs");

    let settled = wait_for("the unexpected exit to be observed", || {
        let status = client
            .get_service_runtime_status(&id)
            .expect("the runtime status is read");
        status
            .process_state
            .is_terminal()
            .then_some(status.process_state)
    });
    assert!(
        matches!(settled, ProcessState::Failed | ProcessState::Stopped),
        "the unexpected exit is recorded as a terminal state, got {settled:?}"
    );

    // The decoy is untouched and the daemon still serves.
    assert!(
        decoy.try_wait().expect("the decoy is waitable").is_none(),
        "the daemon must never signal an unrelated process"
    );
    assert!(
        client.daemon_status().is_ok(),
        "the daemon is still serving"
    );

    let _ = decoy.kill();
    let _ = decoy.wait();
}

/// AC-08/AC-29 — two clients of the same daemon share one authoritative state,
/// and a queued task cancelled by one client is observed as cancelled by the
/// other.
#[test]
fn shared_state_and_queued_task_cancellation_across_clients() {
    let env = Env::start();
    let tui = env.client(ClientKind::Tui);
    let mcp = env.client(ClientKind::Mcp);
    let id = service("orders-api");

    // The TUI starts the service; the other client observes the same state.
    tui.start_service(&id).expect("the start is scheduled");
    wait_for("mcp to observe the shared running state", || {
        let status = mcp
            .get_service_runtime_status(&id)
            .expect("the runtime status is read");
        (status.process_state == ProcessState::Running).then_some(())
    });

    // With `max_parallel_tasks = 1`, the first slow build occupies the slot and
    // the second stays queued.
    let running_build = tui
        .build_service(&service("slow-api"))
        .expect("build scheduled");
    let running_id = running_build.task_id().unwrap().clone();
    wait_for("the first build to start running", || {
        matches!(
            tui.get_task(&running_id).unwrap().status,
            TaskStatus::Running
        )
        .then_some(())
    });

    let queued_build = mcp
        .build_service(&service("slow-api-2"))
        .expect("build scheduled");
    let queued_id = queued_build.task_id().unwrap().clone();

    // The TUI cancels the queued task; the MCP client observes the cancellation.
    tui.cancel_task(&queued_id)
        .expect("cancellation is accepted");
    let cancelled = wait_for_terminal(&mcp, &queued_id);
    assert_eq!(
        cancelled.status,
        TaskStatus::Cancelled,
        "a queued task settles as cancelled"
    );

    // Release the running build so the daemon can drain cleanly.
    let _ = tui.cancel_task(&running_id);
    let _ = tui.stop_service(&id);
}

/// AC-22 — the real MCP stdio server drives the very same daemon an
/// in-process client uses, with pure JSON-RPC on stdout.
#[test]
fn mcp_stdio_drives_the_shared_daemon() {
    let env = Env::start();
    let client = env.client(ClientKind::Tui);
    let id = service("orders-api");

    // Start the service through the in-process client first.
    client.start_service(&id).expect("the start is scheduled");
    wait_for("orders-api to report running", || {
        (client
            .get_service_runtime_status(&id)
            .expect("status")
            .process_state
            == ProcessState::Running)
            .then_some(())
    });

    let mut mcp = McpClient::launch(&env);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "acceptance", "version": "0.0.0" },
        }),
    );
    assert!(init.get("error").is_none(), "initialize succeeds: {init}");
    mcp.notify("notifications/initialized", json!({}));

    // MCP observes the process the other client started.
    let status = envelope(
        &mcp.request(
            "tools/call",
            json!({ "name": "get_service_status", "arguments": { "serviceId": "orders-api" } }),
        )["result"],
    );
    assert_eq!(status["success"], json!(true));
    assert_eq!(
        status["data"]["process_state"],
        json!("running"),
        "MCP observes the shared running state: {status}"
    );

    // A duplicate start via MCP is a no-op, not a second process.
    let started = envelope(
        &mcp.request(
            "tools/call",
            json!({ "name": "start_service", "arguments": { "serviceId": "orders-api" } }),
        )["result"],
    );
    assert_eq!(started["success"], json!(true));
    assert_eq!(
        started["data"]["outcome"],
        json!("already_in_desired_state")
    );

    let code = mcp.shutdown();
    assert_eq!(code, Some(0), "the MCP server exits cleanly on stdin EOF");
    assert!(
        client.daemon_status().is_ok(),
        "the daemon survives the MCP client's exit"
    );
}

/// AC-07/AC-28 — daemon lifecycle: `daemon status`, single-instance
/// enforcement, `daemon stop`, and a reachable socket release.
#[test]
fn daemon_lifecycle_status_single_instance_and_stop() {
    let mut env = Env::start();
    let exe = env!("CARGO_BIN_EXE_service-orchestrator");

    // `daemon status` reports a running daemon without starting one.
    let status = Command::new(exe)
        .args(["daemon", "status", "--config"])
        .arg(&env.config_path)
        .env(SOCKET_ENV, &env.socket)
        .output()
        .expect("daemon status runs");
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status.status.success(), "daemon status exits cleanly");
    assert!(
        stdout.contains("running"),
        "daemon status reports running: {stdout}"
    );

    // A fresh daemon that never started a service adopts nothing.
    let fresh = env.client(ClientKind::Cli);
    let daemon_status = fresh.daemon_status().expect("daemon status is read");
    assert_eq!(daemon_status.managed_services, 0);

    // A second daemon on the same socket must refuse to become authoritative.
    let mut second = Command::new(exe)
        .arg("daemon")
        .arg("--config")
        .arg(&env.config_path)
        .env(SOCKET_ENV, &env.socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the second daemon launches");
    let code = wait_for_exit(&mut second, "second daemon");
    assert_ne!(code, Some(0), "the second daemon refuses to run");
    // The first daemon is unaffected.
    assert!(env.client(ClientKind::Cli).daemon_status().is_ok());

    // `daemon stop` shuts the daemon down and releases the socket.
    let stop = Command::new(exe)
        .args(["daemon", "stop", "--config"])
        .arg(&env.config_path)
        .env(SOCKET_ENV, &env.socket)
        .output()
        .expect("daemon stop runs");
    assert!(stop.status.success(), "daemon stop exits cleanly");
    wait_for_exit(&mut env.daemon, "daemon");
    assert!(
        RemoteClient::connect_existing(&env.config_path, ClientKind::Cli).is_err(),
        "no daemon is reachable after an explicit stop"
    );
}

/// AC-07/AC-28 — a daemon started over a stale socket file and a stale lock
/// file recovers safely and serves normally.
#[test]
fn daemon_recovers_from_a_stale_socket_and_lock_file() {
    let env = Env::start_over_stale_runtime();
    let client = env.client(ClientKind::Cli);
    assert_eq!(client.list_workspaces().expect("workspaces").len(), 1);
    assert!(client.daemon_status().is_ok());
}

// -- small adapters ----------------------------------------------------------

/// The client's Git queries are `async`; the acceptance tests drive them with a
/// tiny current-thread runtime so the call sites stay readable.
fn futures_block<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime is built")
        .block_on(future)
}
