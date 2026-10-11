//! Shared fixtures for the Git adapter integration tests.
//!
//! Every fixture builds a temporary bare "remote" and a working clone under a
//! temporary directory, then drives the adapter exactly as the application
//! would. No network, GitHub, SSH, or developer repository is involved
//! (`tasks/007.md` §23).
//!
//! Integration tests in a Cargo crate are separate binaries, so this module is
//! compiled into each of them; not every helper is used by every test file,
//! hence the module-level `dead_code` allowance.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use service_orchestrator_application::{GitOptions, ResolvedRepository};
use service_orchestrator_domain::{AbsolutePath, Branch, WorkspaceId};
use service_orchestrator_git::CliGitManager;
use service_orchestrator_process::ProcessManager;

/// A temporary directory removed when the fixture is dropped.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh, uniquely named temporary directory.
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("so-git-{label}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temporary directory is creatable");
        Self { path }
    }

    /// Returns the temporary directory path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Runs `git` in `dir` with a deterministic, isolated environment.
pub(crate) fn git_output(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("LC_ALL", "C")
        .output()
        .expect("git is installed and runnable")
}

/// Runs `git`, asserting that it succeeds.
pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_output(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A workspace with one remote repository and a working clone.
pub(crate) struct Fixture {
    _root: TempDir,
    /// The workspace root that contains the checkout.
    pub(crate) workspace_root: PathBuf,
    /// The bare remote repository.
    pub(crate) remote: PathBuf,
    /// A separate working clone used to push new commits to the remote.
    pub(crate) seed: PathBuf,
    /// The checkout under test, inside the workspace root.
    pub(crate) checkout: PathBuf,
    /// The default branch of the remote.
    pub(crate) branch: String,
}

impl Fixture {
    /// Builds a remote, a seed clone, and a working clone for `branch`.
    pub(crate) fn new(branch: &str) -> Self {
        Self::named(branch, "auth-service")
    }

    /// Builds a fixture whose checkout directory is `name` under the workspace.
    pub(crate) fn named(branch: &str, name: &str) -> Self {
        let root = TempDir::new("fixture");
        let workspace_root = root.path.join("workspace");
        let remote = root.path.join("remote.git");
        let seed = root.path.join("seed");
        let checkout = workspace_root.join("backend").join(name);

        std::fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "--bare", "-b", branch]);

        std::fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-b", branch]);
        std::fs::write(seed.join("README.md"), "hello\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-m", "initial"]);
        git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&seed, &["push", "-u", "origin", branch]);

        std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
        git(
            &workspace_root,
            &[
                "clone",
                remote.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
        );

        Self {
            _root: root,
            workspace_root,
            remote,
            seed,
            checkout,
            branch: branch.to_owned(),
        }
    }

    /// Returns the resolved repository the application would pass to the port.
    pub(crate) fn repository(&self) -> ResolvedRepository {
        self.repository_named("auth-service")
    }

    /// Returns the resolved repository under a chosen workspace-relative name.
    pub(crate) fn repository_named(&self, name: &str) -> ResolvedRepository {
        ResolvedRepository {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: name.to_owned(),
            url: self.remote.to_string_lossy().into_owned(),
            path: AbsolutePath::new(self.checkout.to_string_lossy().into_owned()).unwrap(),
            workspace_root: AbsolutePath::new(self.workspace_root.to_string_lossy().into_owned())
                .unwrap(),
            default_branch: Branch::new(&self.branch).unwrap(),
        }
    }

    /// Commits and pushes a new file to the remote's default branch.
    pub(crate) fn remote_commit(&self, name: &str) {
        std::fs::write(self.seed.join(format!("{name}.txt")), "content\n").unwrap();
        git(&self.seed, &["add", "."]);
        git(&self.seed, &["commit", "-m", name]);
        git(&self.seed, &["push", "origin", &self.branch]);
    }

    /// Creates and pushes a new branch on the remote.
    pub(crate) fn remote_branch(&self, name: &str) {
        git(&self.seed, &["checkout", "-b", name]);
        std::fs::write(self.seed.join("branch.txt"), "branch\n").unwrap();
        git(&self.seed, &["add", "."]);
        git(&self.seed, &["commit", "-m", name]);
        git(&self.seed, &["push", "-u", "origin", name]);
        git(&self.seed, &["checkout", &self.branch]);
    }
}

/// Creates a manager backed by a fresh process manager.
pub(crate) fn manager() -> CliGitManager {
    CliGitManager::new(ProcessManager::new().unwrap())
}

/// Default options for a mutating operation.
pub(crate) fn options() -> GitOptions {
    GitOptions::new()
}
