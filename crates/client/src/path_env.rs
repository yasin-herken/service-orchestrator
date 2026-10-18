//! A safe, static `PATH` augmentation for the daemon auto-started by a GUI.
//!
//! An application launched from Finder does not inherit a login shell's
//! environment, so its `PATH` is the bare system default and misses Homebrew,
//! SDKMAN, nvm, fnm, Volta, and the other places macOS developers install
//! `git`, `java`, `mvn`, `node`, and `npm`. When the desktop GUI is the process
//! that auto-starts the daemon, the daemon — and every managed service it later
//! spawns — would inherit that crippled `PATH` and fail to find its toolchain
//! (`tasks/031.md` §19).
//!
//! This module closes that gap the conservative way the task requires: it
//! **discovers well-known directories on disk** and prepends their existing
//! entries to the `PATH` the daemon is spawned with. It never runs a login
//! shell, never sources a shell profile, and never guesses a directory that is
//! not actually present. Explicit configuration (a `JAVA_HOME`, an absolute
//! command path, a `mvnw` wrapper) still wins because the augmented entries are
//! appended after the inherited ones and only when they are genuinely missing.
//!
//! The same directory list is used by `crates/runtime`'s
//! `SystemRuntimeHost::search_directories` for runtime detection, so discovery
//! and execution agree on where tools live.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Fixed directories that commonly hold toolchains on macOS, searched in this
/// order. They are only added when they exist.
const FIXED_DIRECTORIES: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

/// Home-relative directories used by common version managers.
const HOME_DIRECTORIES: &[&str] = &[
    ".volta/bin",
    ".asdf/shims",
    ".local/share/fnm/aliases/default/bin",
    ".sdkman/candidates/java/current/bin",
    ".jenv/shims",
];

/// Merges `additional` directories into `existing`, preserving order.
///
/// The inherited entries always come first, so an explicitly configured
/// toolchain keeps precedence. An additional entry is appended only when it is
/// a non-empty, existing directory that is not already present.
#[must_use]
pub fn merge_search_path(existing: Option<&OsStr>, additional: &[PathBuf]) -> OsString {
    let mut entries: Vec<PathBuf> = existing
        .map(|value| {
            std::env::split_paths(value)
                // An empty entry means "the current directory" to a shell; it
                // must never be searched for a managed-service toolchain.
                .filter(|entry| !entry.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    for directory in additional {
        if directory.as_os_str().is_empty() {
            continue;
        }
        if directory.is_dir() && !entries.contains(directory) {
            entries.push(directory.clone());
        }
    }
    std::env::join_paths(entries)
        .unwrap_or_else(|_| existing.map_or_else(OsString::new, OsString::from))
}

/// Returns the tool directories this machine actually has, in search order.
///
/// The list mirrors `SystemRuntimeHost::search_directories` in
/// `crates/runtime`: `JAVA_HOME/bin`, the version managers, the nvm Node bins
/// (newest first), then the fixed system/Homebrew directories.
#[must_use]
pub fn discovered_tool_directories(home: Option<&Path>, java_home: Option<&Path>) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = Vec::new();

    if let Some(java_home) = java_home {
        directories.push(java_home.join("bin"));
    }
    if let Some(home) = home {
        for suffix in HOME_DIRECTORIES {
            directories.push(home.join(suffix));
        }
        // nvm installs one bin directory per Node version; newest first.
        let versions = home.join(".nvm/versions/node");
        if let Ok(read) = std::fs::read_dir(versions) {
            let mut bins: Vec<PathBuf> = read
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("bin"))
                .collect();
            bins.sort();
            bins.reverse();
            directories.extend(bins);
        }
    }
    for fixed in FIXED_DIRECTORIES {
        directories.push(PathBuf::from(fixed));
    }
    directories
}

/// Computes the augmented `PATH` for a daemon spawned from this process.
///
/// Reads the live `PATH`, `HOME`, and `JAVA_HOME` and appends every discovered
/// tool directory that exists. It is the value exported to the auto-started
/// daemon, never the value this process uses itself.
#[must_use]
pub fn effective_search_path() -> OsString {
    let existing = std::env::var_os("PATH");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    let candidates = discovered_tool_directories(home.as_deref(), java_home.as_deref());
    merge_search_path(existing.as_deref(), &candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a scratch directory that cleans itself up.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "service-orchestrator-path-env-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir exists");
        dir
    }

    fn entries(value: &OsStr) -> Vec<PathBuf> {
        std::env::split_paths(value).collect()
    }

    #[test]
    fn inherited_entries_come_first_and_are_never_dropped() {
        let dir = scratch("order");
        let inherited = dir.join("inherited");
        let extra = dir.join("extra");
        std::fs::create_dir_all(&inherited).expect("inherited exists");
        std::fs::create_dir_all(&extra).expect("extra exists");

        let merged = merge_search_path(Some(inherited.as_os_str()), std::slice::from_ref(&extra));
        assert_eq!(entries(&merged), vec![inherited, extra]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_directories_are_skipped() {
        let dir = scratch("missing");
        let real = dir.join("real");
        std::fs::create_dir_all(&real).expect("real exists");
        let absent = dir.join("absent");

        let merged = merge_search_path(None, &[absent, real.clone()]);
        assert_eq!(entries(&merged), vec![real]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicates_are_not_added_twice() {
        let dir = scratch("dedupe");
        let shared = dir.join("shared");
        std::fs::create_dir_all(&shared).expect("shared exists");

        let merged = merge_search_path(Some(shared.as_os_str()), std::slice::from_ref(&shared));
        assert_eq!(entries(&merged), vec![shared]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_empty_inherited_path_still_gets_the_discovered_entries() {
        let dir = scratch("empty");
        let extra = dir.join("extra");
        std::fs::create_dir_all(&extra).expect("extra exists");

        let merged = merge_search_path(Some(OsStr::new("")), std::slice::from_ref(&extra));
        // split_paths("") yields no entries, so only the discovered one remains.
        assert_eq!(entries(&merged), vec![extra]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn discovery_includes_java_home_and_home_relative_managers() {
        let home = scratch("home");
        let java = home.join("jdk");

        let directories = discovered_tool_directories(Some(&home), Some(&java));
        assert!(directories.contains(&java.join("bin")));
        assert!(directories.contains(&home.join(".volta/bin")));
        // The nvm *parent* is not a bin directory, so it is never added.
        assert!(!directories.contains(&home.join(".nvm/versions/node")));
        assert!(directories.contains(&PathBuf::from("/opt/homebrew/bin")));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn nvm_node_bins_are_discovered_newest_first() {
        let home = scratch("nvm");
        for version in ["v18.20.0", "v22.18.0", "v20.x"] {
            std::fs::create_dir_all(home.join(".nvm/versions/node").join(version).join("bin"))
                .expect("nvm version exists");
        }

        let directories = discovered_tool_directories(Some(&home), None);
        let nvm: Vec<PathBuf> = directories
            .into_iter()
            .filter(|path| path.starts_with(home.join(".nvm/versions/node")))
            .collect();
        assert_eq!(
            nvm,
            vec![
                home.join(".nvm/versions/node/v22.18.0/bin"),
                home.join(".nvm/versions/node/v20.x/bin"),
                home.join(".nvm/versions/node/v18.20.0/bin"),
            ],
            "newest Node version is searched first"
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn effective_search_path_keeps_the_live_path() {
        // The live PATH entries survive the augmentation (nothing is dropped),
        // so an explicitly configured toolchain still wins.
        let live = std::env::var_os("PATH").expect("PATH is set in the test environment");
        let augmented = effective_search_path();
        let augmented_entries = entries(&augmented);
        for entry in entries(&live) {
            assert!(
                augmented_entries.contains(&entry),
                "inherited entry {} survives",
                entry.display()
            );
        }
        // The inherited entries are a prefix of the result.
        assert_eq!(
            &augmented_entries[..entries(&live).len()],
            &entries(&live)[..]
        );
    }
}
