//! Locating the `service-orchestrator` executable that daemon autostart spawns.
//!
//! The GUI process has a different `current_exe()` than the shared client
//! library expects, so the adapter resolves the real host binary and exports
//! it through [`DAEMON_BIN_ENV`] before connecting
//! (`tasks/018.md` §14, `crates/client/src/socket.rs`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use service_orchestrator_client::DAEMON_BIN_ENV;

/// The executable name of the daemon host binary.
pub const DAEMON_BINARY: &str = "service-orchestrator";

/// Resolves the daemon executable for a host program.
///
/// Resolution order:
///
/// 1. an explicit override (the [`DAEMON_BIN_ENV`] environment variable), used
///    as-is only when it points at an existing file;
/// 2. `PATH`;
/// 3. a sibling of `current_exe` (installed or `cargo build` layout);
/// 4. `target/{debug,release}` directories walking up from `current_exe`,
///    which covers the `npm run tauri dev` layout where the GUI binary lives
///    under `apps/desktop/src-tauri/target/debug/` while the shared binary is
///    built in the repository's root `target/`.
///
/// Returns [`None`] when no candidate exists, so the caller can produce one
/// actionable error instead of spawning a doomed process.
#[must_use]
pub fn resolve_daemon_binary(
    current_exe: &Path,
    explicit_override: Option<&Path>,
    search_path: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(path) = explicit_override {
        return path.is_file().then(|| path.to_path_buf());
    }
    if let Some(raw_path) = search_path {
        for directory in std::env::split_paths(raw_path) {
            let candidate = directory.join(DAEMON_BINARY);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let executable_dir = current_exe.parent()?;
    let sibling = executable_dir.join(DAEMON_BINARY);
    if sibling.is_file() {
        return Some(sibling);
    }
    for ancestor in current_exe.ancestors().skip(1) {
        for profile in ["debug", "release"] {
            let candidate = ancestor.join("target").join(profile).join(DAEMON_BINARY);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolves the daemon binary from this process's live environment.
///
/// Returns [`None`] when the binary cannot be found anywhere; the connection
/// attempt then fails with a single actionable message naming every remedy.
#[must_use]
pub fn resolve_from_environment() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let explicit = std::env::var_os(DAEMON_BIN_ENV).map(PathBuf::from);
    let search_path = std::env::var_os("PATH");
    resolve_daemon_binary(&current_exe, explicit.as_deref(), search_path.as_deref())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Creates a scratch directory that cleans itself up.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "service-orchestrator-daemon-bin-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir exists");
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir exists");
        }
        fs::write(path, b"#! /bin/sh\n").expect("file is written");
    }

    #[test]
    fn an_explicit_override_wins_when_it_exists() {
        let dir = scratch("override");
        let explicit = dir.join("custom-daemon");
        touch(&explicit);

        let resolved = resolve_daemon_binary(Path::new("/nonexistent/app"), Some(&explicit), None);
        assert_eq!(resolved, Some(explicit));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_override_yields_nothing_instead_of_a_bad_spawn() {
        let dir = scratch("override-missing");
        let resolved = resolve_daemon_binary(
            Path::new("/nonexistent/app"),
            Some(&dir.join("absent")),
            None,
        );
        assert_eq!(resolved, None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn path_is_searched_before_sibling_locations() {
        let dir = scratch("path");
        let bin_dir = dir.join("bin");
        let on_path = bin_dir.join(DAEMON_BINARY);
        touch(&on_path);
        let app_dir = dir.join("app");
        touch(&app_dir.join("gui-app"));
        // No sibling daemon next to the app: PATH must find it.
        let resolved =
            resolve_daemon_binary(&app_dir.join("gui-app"), None, Some(bin_dir.as_os_str()));
        assert_eq!(resolved, Some(on_path));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_sibling_daemon_is_found_next_to_the_program() {
        let dir = scratch("sibling");
        let app_dir = dir.join("app");
        let sibling = app_dir.join(DAEMON_BINARY);
        touch(&sibling);
        touch(&app_dir.join("gui-app"));

        let resolved = resolve_daemon_binary(&app_dir.join("gui-app"), None, None);
        assert_eq!(resolved, Some(sibling));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn target_directories_are_walked_up_from_a_nested_binary() {
        let dir = scratch("walkup");
        // npm run tauri dev layout: apps/desktop/src-tauri/target/debug/app
        let nested = dir
            .join("apps")
            .join("desktop")
            .join("src-tauri")
            .join("target")
            .join("debug");
        touch(&nested.join("gui-app"));
        let repo_target = dir.join("target").join("debug");
        let shared = repo_target.join(DAEMON_BINARY);
        touch(&shared);

        let resolved = resolve_daemon_binary(&nested.join("gui-app"), None, None);
        assert_eq!(resolved, Some(shared));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_found_is_reported_honestly() {
        let dir = scratch("absent");
        let resolved = resolve_daemon_binary(&dir.join("gui-app"), None, None);
        assert_eq!(resolved, None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_env_override_name_is_the_client_contract() {
        assert_eq!(DAEMON_BIN_ENV, "SERVICE_ORCHESTRATOR_DAEMON_BIN");
        assert_eq!(DAEMON_BINARY, "service-orchestrator");
    }
}
