//! Pure Git command construction.
//!
//! Every argument vector is built here as plain data and tested in isolation.
//! Values that originate from configuration or user input (branch names,
//! repository URLs) are passed as separate arguments — never interpolated into
//! a shell string — and, where Git supports it, after a `--` separator so a
//! value can never be mistaken for an option (`AGENTS.md` §17, §32).
//!
//! Branch names are already validated by the domain ([`Branch`]) to be
//! non-empty, free of whitespace, and not to begin with `-`, which removes the
//! option-injection risk for revision arguments.
//!
//! [`Branch`]: service_orchestrator_domain::Branch

/// The Git executable resolved from `PATH`.
pub(crate) const GIT_PROGRAM: &str = "git";

/// The remote name used when fetching, pulling, and setting up tracking.
///
/// Repository configuration does not model a remote name yet; a clone created
/// by [`clone_args`] names its remote `origin`, which is the Git default.
pub(crate) const DEFAULT_REMOTE: &str = "origin";

/// The prefix used for remote-tracking reference names.
pub(crate) const REMOTE_PREFIX: &str = "origin";

/// Machine-readable status with branch headers and NUL separators.
pub(crate) fn status_args() -> Vec<String> {
    vec![
        "status".to_owned(),
        "--porcelain=v2".to_owned(),
        "--branch".to_owned(),
        "-z".to_owned(),
    ]
}

/// Machine-readable branch listing: `refname`, `refname:short`, `objectname`,
/// `HEAD`, and `upstream:short`, tab-separated.
///
/// `git branch` separates records with newlines; branch names and object names
/// cannot contain a tab, so `\t` is an unambiguous field separator.
pub(crate) fn branch_list_args() -> Vec<String> {
    vec![
        "branch".to_owned(),
        "--all".to_owned(),
        "--format=%(refname)%09%(refname:short)%09%(objectname:short)%09%(HEAD)%09%(upstream:short)"
            .to_owned(),
    ]
}

/// The `git diff` arguments for the working tree (`staged == false`) or the
/// index (`staged == true`), without a patch-format flag.
pub(crate) fn diff_base_args(staged: bool) -> Vec<String> {
    let mut args = vec!["diff".to_owned(), "--no-color".to_owned()];
    if staged {
        args.push("--cached".to_owned());
    }
    args
}

/// A machine-readable per-file diff summary (`--numstat`).
pub(crate) fn diff_numstat_args(staged: bool) -> Vec<String> {
    let mut args = diff_base_args(staged);
    args.push("--numstat".to_owned());
    args
}

/// A unified patch with three lines of context, written to `output`.
///
/// The patch is redirected to a file rather than captured from standard output
/// so an arbitrarily large diff never has to travel through the process
/// manager's bounded line-event channel; the manager then reads back only the
/// bounded prefix it returns (`AGENTS.md` §40).
pub(crate) fn diff_patch_args(staged: bool, output: &str) -> Vec<String> {
    let mut args = diff_base_args(staged);
    args.push("--unified=3".to_owned());
    args.push(format!("--output={output}"));
    args
}

/// Verifies the current commit, failing on an unborn branch.
pub(crate) fn rev_parse_head_args() -> Vec<String> {
    vec![
        "rev-parse".to_owned(),
        "--verify".to_owned(),
        "--quiet".to_owned(),
        "HEAD".to_owned(),
    ]
}

/// Checks whether a local branch reference exists.
pub(crate) fn verify_local_branch_args(branch: &str) -> Vec<String> {
    vec![
        "rev-parse".to_owned(),
        "--verify".to_owned(),
        "--quiet".to_owned(),
        format!("refs/heads/{branch}"),
    ]
}

/// Checks whether a remote-tracking branch reference exists.
pub(crate) fn verify_remote_branch_args(branch: &str) -> Vec<String> {
    vec![
        "rev-parse".to_owned(),
        "--verify".to_owned(),
        "--quiet".to_owned(),
        format!("refs/remotes/{REMOTE_PREFIX}/{branch}"),
    ]
}

/// Fetches a remote, optionally pruning deleted remote-tracking references.
pub(crate) fn fetch_args(prune: bool) -> Vec<String> {
    let mut args = vec!["fetch".to_owned(), DEFAULT_REMOTE.to_owned()];
    if prune {
        args.push("--prune".to_owned());
    }
    args
}

/// Pulls the current branch, refusing anything but a fast-forward.
pub(crate) fn pull_args() -> Vec<String> {
    vec!["pull".to_owned(), "--ff-only".to_owned()]
}

/// Checks out an existing local branch.
pub(crate) fn checkout_args(branch: &str) -> Vec<String> {
    vec!["checkout".to_owned(), branch.to_owned()]
}

/// Creates a local branch tracking the matching remote-tracking branch.
pub(crate) fn checkout_track_args(branch: &str) -> Vec<String> {
    vec![
        "checkout".to_owned(),
        "--track".to_owned(),
        "-b".to_owned(),
        branch.to_owned(),
        format!("{REMOTE_PREFIX}/{branch}"),
    ]
}

/// Clones a repository at a branch into a destination directory.
pub(crate) fn clone_args(url: &str, branch: &str, destination: &str) -> Vec<String> {
    vec![
        "clone".to_owned(),
        "--branch".to_owned(),
        branch.to_owned(),
        "--".to_owned(),
        url.to_owned(),
        destination.to_owned(),
    ]
}

/// Discards tracked modifications with a hard reset to `HEAD`.
pub(crate) fn reset_hard_args() -> Vec<String> {
    vec!["reset".to_owned(), "--hard".to_owned(), "HEAD".to_owned()]
}

/// Removes untracked files and directories.
pub(crate) fn clean_args() -> Vec<String> {
    vec!["clean".to_owned(), "-fd".to_owned()]
}

/// Inherited environment variables removed from every Git process.
///
/// The daemon's environment is inherited by child processes. A value such as
/// `GIT_DIR` or `GIT_WORK_TREE` in that environment would make Git operate on a
/// repository other than the resolved checkout, and `GIT_SSH_COMMAND`,
/// `GIT_EXTERNAL_DIFF`, or a substituted `GIT_CONFIG` could make Git execute
/// arbitrary code. Removing them keeps every Git operation scoped to the
/// repository the application resolved (`AGENTS.md` §21, §32).
///
/// The developer's global and system Git configuration is deliberately left in
/// place so existing authentication (credential helpers, SSH configuration) and
/// `includeIf` behavior keep working.
pub(crate) const HARDENED_ENVIRONMENT_REMOVALS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_EXTERNAL_DIFF",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
];

/// Non-interactive environment applied to every Git process.
///
/// Without these, Git can block forever waiting for a terminal credential
/// prompt, which would hang an MCP or TUI operation with no way to answer it
/// (`AGENTS.md` §9). The developer's existing SSH agent and credential helpers
/// are untouched.
pub(crate) fn non_interactive_environment() -> service_orchestrator_domain::Environment {
    use service_orchestrator_domain::{Environment, EnvironmentVariable};

    let mut environment = Environment::new();
    for (key, value) in [
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_ASKPASS", ""),
        ("SSH_ASKPASS", ""),
        ("GCM_INTERACTIVE", "never"),
    ] {
        // Static, valid keys and values; construction cannot fail.
        let variable = EnvironmentVariable::new(key, value)
            .expect("static Git environment variables are valid");
        environment
            .set(variable)
            .expect("static Git environment variables are valid");
    }
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_a_machine_readable_format() {
        assert_eq!(
            status_args(),
            vec!["status", "--porcelain=v2", "--branch", "-z"]
        );
    }

    #[test]
    fn branch_names_are_separate_arguments() {
        let args = checkout_args("feature/x");
        assert_eq!(args, vec!["checkout", "feature/x"]);
        // The branch is its own argument, never part of a shell fragment.
        assert!(!args.iter().any(|arg| arg.contains(' ')));
    }

    #[test]
    fn clone_uses_the_path_separator() {
        let args = clone_args("git@github.com:acme/app.git", "main", "/w/app");
        assert_eq!(
            args,
            vec![
                "clone",
                "--branch",
                "main",
                "--",
                "git@github.com:acme/app.git",
                "/w/app"
            ]
        );
        assert_eq!(args[3], "--");
    }

    #[test]
    fn pull_is_fast_forward_only() {
        assert_eq!(pull_args(), vec!["pull", "--ff-only"]);
    }

    #[test]
    fn fetch_prunes_only_when_requested() {
        assert_eq!(fetch_args(false), vec!["fetch", "origin"]);
        assert_eq!(fetch_args(true), vec!["fetch", "origin", "--prune"]);
    }

    #[test]
    fn diff_stages_the_index_only_when_requested() {
        assert_eq!(
            diff_numstat_args(false),
            vec!["diff", "--no-color", "--numstat"]
        );
        assert_eq!(
            diff_numstat_args(true),
            vec!["diff", "--no-color", "--cached", "--numstat"]
        );
    }

    #[test]
    fn diff_patch_is_written_to_a_file() {
        assert_eq!(
            diff_patch_args(false, "/tmp/so.patch"),
            vec![
                "diff",
                "--no-color",
                "--unified=3",
                "--output=/tmp/so.patch"
            ]
        );
        assert_eq!(
            diff_patch_args(true, "/tmp/so.patch"),
            vec![
                "diff",
                "--no-color",
                "--cached",
                "--unified=3",
                "--output=/tmp/so.patch"
            ]
        );
    }

    #[test]
    fn git_environment_is_non_interactive() {
        let environment = non_interactive_environment();
        assert_eq!(environment.get("GIT_TERMINAL_PROMPT").unwrap().value, "0");
        assert!(environment.contains("GIT_ASKPASS"));
        assert!(environment.contains("SSH_ASKPASS"));
    }

    #[test]
    fn hardened_removals_cover_repository_redirection_and_code_execution() {
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_SSH_COMMAND",
            "GIT_EXTERNAL_DIFF",
        ] {
            assert!(
                HARDENED_ENVIRONMENT_REMOVALS.contains(&name),
                "{name} must be removed from the Git environment"
            );
        }
    }
}
