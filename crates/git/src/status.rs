//! Parsing of machine-readable `git status` output.
//!
//! The manager runs `git status --porcelain=v2 --branch -z` and hands the raw
//! `stdout` here. Porcelain v2 is the stable, documented, non-localized format
//! (`AGENTS.md` §19): every record is NUL-separated and its shape is fixed, so
//! filenames containing spaces, unicode, or newlines cannot confuse the parser.
//!
//! # Record shapes
//!
//! ```text
//! # branch.oid <sha> | (initial)
//! # branch.head <name> | (detached)
//! # branch.upstream <upstream-branch>
//! # branch.ab +<ahead> -<behind>
//! 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
//! 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>
//! u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
//! ? <path>
//! ! <path>
//! ```
//!
//! Ordinary (`1`) and renamed/copied (`2`) entries carry the two-character
//! `XY` field: `X` is the index (staged) state and `Y` the working-tree
//! (unstaged) state, `.` meaning unchanged. Unmerged entries (`u`) are counted
//! as conflicts; untracked entries (`?`) as untracked files; ignored entries
//! (`!`) are not listed because `git status` is not asked for them.
//!
//! A rename record is the one case that spans *two* NUL-separated tokens: the
//! record itself and the original path. The parser consumes the extra token so
//! the following record is not misread as a path.

use service_orchestrator_domain::{Branch, RepositoryLifecycle, RepositoryStatus};

/// Parses porcelain v2 `--branch` output into a structured status.
///
/// The returned status always has lifecycle [`Available`](RepositoryLifecycle::Available);
/// the manager decides lifecycle before invoking the parser (a missing path is
/// reported as [`not_cloned`](RepositoryStatus::not_cloned), a path that is not
/// a working copy as [`Invalid`](RepositoryLifecycle::Invalid)).
#[must_use]
pub(crate) fn parse_status(output: &str) -> RepositoryStatus {
    let mut status = RepositoryStatus {
        lifecycle: RepositoryLifecycle::Available,
        ..RepositoryStatus::default()
    };

    // Empty tokens are only ever the separators between records (including the
    // trailing NUL); skipping them cannot hide a rename's original path, which
    // is always non-empty.
    let mut records = output.split('\0').filter(|token| !token.is_empty());

    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("# ") {
            parse_header(header, &mut status);
            continue;
        }
        match record.as_bytes().first().copied() {
            Some(b'1') => count_entry(record, &mut status),
            Some(b'2') => {
                // A rename/copy record is followed by its original path.
                count_entry(record, &mut status);
                let _ = records.next();
            }
            Some(b'u') => {
                status.conflicts += 1;
            }
            Some(b'?') => {
                status.untracked_files += 1;
            }
            // `!` (ignored) and anything unexpected are ignored rather than
            // guessed at.
            _ => {}
        }
    }

    status
}

/// Applies a `# branch.*` header to the in-progress status.
fn parse_header(header: &str, status: &mut RepositoryStatus) {
    if let Some(oid) = header.strip_prefix("branch.oid ") {
        if oid == "(initial)" {
            status.unborn = true;
        } else if !oid.is_empty() {
            status.commit = Some(oid.to_owned());
        }
        return;
    }
    if let Some(head) = header.strip_prefix("branch.head ") {
        if head == "(detached)" {
            status.detached_head = true;
        } else if let Ok(branch) = Branch::new(head) {
            status.branch = Some(branch);
        }
        return;
    }
    if let Some(upstream) = header.strip_prefix("branch.upstream ") {
        if !upstream.is_empty() {
            status.upstream = Some(upstream.to_owned());
        }
        return;
    }
    if let Some(ahead_behind) = header.strip_prefix("branch.ab ") {
        for part in ahead_behind.split_whitespace() {
            if let Some(value) = part.strip_prefix('+') {
                status.ahead = value.parse().ok();
            } else if let Some(value) = part.strip_prefix('-') {
                status.behind = value.parse().ok();
            }
        }
    }
}

/// Counts the staged and unstaged parts of an ordinary or rename record.
fn count_entry(record: &str, status: &mut RepositoryStatus) {
    // The `XY` field is the second whitespace-separated token and is exactly two
    // bytes: `X` (index) and `Y` (work tree).
    let Some(xy) = record.split(' ').nth(1) else {
        return;
    };
    let mut chars = xy.chars();
    if let Some(index) = chars.next() {
        if index != '.' {
            status.staged_changes += 1;
        }
    }
    if let Some(worktree) = chars.next() {
        if worktree != '.' {
            status.unstaged_changes += 1;
        }
    }
}

/// Parses `git branch --format=%(refname)%09...` output into branch listings.
///
/// Each line has five tab-separated fields: full refname, short name, short
/// object name, `HEAD` marker, and upstream short name. The synthetic
/// `refs/remotes/<remote>/HEAD` alias is skipped.
#[must_use]
pub(crate) fn parse_branches(output: &str) -> Vec<service_orchestrator_domain::BranchInfo> {
    use service_orchestrator_domain::{BranchInfo, BranchKind};

    output
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let mut fields = line.split('\t');
            let refname = fields.next()?.trim();
            let name = fields.next().unwrap_or_default().trim();
            let commit = fields.next().unwrap_or_default().trim();
            let head = fields.next().unwrap_or_default().trim();
            let upstream = fields.next().unwrap_or_default().trim();

            let kind = if refname.starts_with("refs/heads/") {
                BranchKind::Local
            } else if refname.starts_with("refs/remotes/") {
                BranchKind::Remote
            } else {
                return None;
            };
            // `origin/HEAD` is a symbolic alias, not a real branch.
            if name.ends_with("/HEAD") {
                return None;
            }
            let mut info = BranchInfo::new(name, kind, head == "*");
            if !upstream.is_empty() {
                info.upstream = Some(upstream.to_owned());
            }
            if !commit.is_empty() {
                info.commit = Some(commit.to_owned());
            }
            Some(info)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_clean_repository() {
        let output = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\0";
        let status = parse_status(output);
        assert_eq!(status.lifecycle, RepositoryLifecycle::Available);
        assert_eq!(status.branch.as_ref().map(Branch::as_str), Some("main"));
        assert_eq!(status.commit.as_deref(), Some("abc123"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.ahead, Some(0));
        assert_eq!(status.behind, Some(0));
        assert!(status.is_clean());
        assert!(!status.detached_head);
        assert!(!status.unborn);
    }

    #[test]
    fn counts_staged_unstaged_untracked_and_conflicts() {
        let output = concat!(
            "# branch.oid abc\0# branch.head main\0",
            "1 M. N... 100644 100644 100644 aaa bbb src/staged.rs\0",
            "1 .M N... 100644 100644 100644 aaa bbb src/unstaged.rs\0",
            "1 MM N... 100644 100644 100644 aaa bbb src/both.rs\0",
            "? untracked file.txt\0",
            "u UU N... 100644 100644 100644 100644 aaa bbb ccc src/conflict.rs\0",
        );
        let status = parse_status(output);
        // staged: src/staged.rs, src/both.rs
        assert_eq!(status.staged_changes, 2);
        // unstaged: src/unstaged.rs, src/both.rs
        assert_eq!(status.unstaged_changes, 2);
        assert_eq!(status.untracked_files, 1);
        assert_eq!(status.conflicts, 1);
        assert!(!status.is_clean());
        assert!(status.has_local_changes());
    }

    #[test]
    fn a_rename_record_consumes_its_original_path() {
        let output = concat!(
            "# branch.oid abc\0# branch.head main\0",
            "2 R. N... 100644 100644 100644 aaa bbb R100 src/new.rs\0src/old.rs\0",
            "? later.txt\0",
        );
        let status = parse_status(output);
        assert_eq!(status.staged_changes, 1);
        assert_eq!(status.unstaged_changes, 0);
        // If the original path were misread as a record, this count would differ.
        assert_eq!(status.untracked_files, 1);
    }

    #[test]
    fn parses_detached_head() {
        let output = "# branch.oid abc\0# branch.head (detached)\0";
        let status = parse_status(output);
        assert!(status.detached_head);
        assert!(status.branch.is_none());
        assert_eq!(status.commit.as_deref(), Some("abc"));
    }

    #[test]
    fn parses_an_unborn_branch() {
        let output = "# branch.oid (initial)\0# branch.head main\0";
        let status = parse_status(output);
        assert!(status.unborn);
        assert!(status.commit.is_none());
        assert_eq!(status.branch.as_ref().map(Branch::as_str), Some("main"));
    }

    #[test]
    fn parses_ahead_and_behind() {
        let output = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +3 -2\0";
        let status = parse_status(output);
        assert!(status.is_ahead());
        assert!(status.is_behind());
        assert!(status.is_diverged());
    }

    #[test]
    fn missing_upstream_leaves_counts_unknown() {
        let output = "# branch.oid abc\0# branch.head main\0";
        let status = parse_status(output);
        assert!(status.upstream.is_none());
        assert_eq!(status.ahead, None);
        assert_eq!(status.behind, None);
    }

    #[test]
    fn parses_branch_listings_and_skips_the_remote_head_alias() {
        let output = concat!(
            "refs/heads/main\tmain\tabc123\t*\torigin/main\n",
            "refs/heads/feature/x\tfeature/x\tdef456\t\t\n",
            "refs/remotes/origin/main\torigin/main\tabc123\t\t\n",
            "refs/remotes/origin/HEAD\torigin/HEAD\tabc123\t\t\n",
        );
        let branches = parse_branches(output);
        assert_eq!(branches.len(), 3);
        let main = &branches[0];
        assert_eq!(main.name, "main");
        assert!(main.current);
        assert_eq!(main.upstream.as_deref(), Some("origin/main"));
        assert_eq!(main.commit.as_deref(), Some("abc123"));
        use service_orchestrator_domain::BranchKind;
        assert_eq!(main.kind, BranchKind::Local);
        assert_eq!(branches[2].kind, BranchKind::Remote);
        assert!(!branches[2].current);
    }
}
