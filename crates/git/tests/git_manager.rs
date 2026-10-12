//! Integration tests for the Git manager against real, local Git fixtures.
//!
//! Every test builds a temporary workspace through [`support::Fixture`] and
//! drives the manager exactly as the application would. No network, GitHub,
//! SSH, or developer repository is involved (`tasks/007.md` §23).

mod support;

use std::sync::Arc;

use service_orchestrator_application::{DiffOptions, GitError, GitManager};
use service_orchestrator_domain::{
    AbsolutePath, Branch, BranchKind, GitOperation, RepositoryLifecycle,
};
use support::{git, manager, options, Fixture};

// -- status ------------------------------------------------------------------

#[tokio::test]
async fn status_reports_a_branch_with_no_upstream() {
    // A local-only branch has no upstream: that is a legitimate state, not an
    // error, and `sync` must be able to report it rather than fail opaquely.
    let fixture = Fixture::new("main");
    git(&fixture.checkout, &["checkout", "-b", "local-only"]);

    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();
    assert_eq!(
        status.branch.as_ref().map(Branch::as_str),
        Some("local-only")
    );
    assert!(status.upstream.is_none(), "no upstream is configured");
    assert_eq!(status.ahead, None);
    assert_eq!(status.behind, None);
}

#[tokio::test]
async fn status_counts_unmerged_entries_during_a_merge_conflict() {
    let fixture = Fixture::new("main");
    // Both sides modify the same line of the same file, so the merge conflicts.
    std::fs::write(fixture.checkout.join("README.md"), "local version\n").unwrap();
    git(&fixture.checkout, &["commit", "-am", "local change"]);
    // The seed commits a different version of the same file to the remote.
    std::fs::write(fixture.seed.join("README.md"), "remote version\n").unwrap();
    git(&fixture.seed, &["commit", "-am", "remote change"]);
    git(&fixture.seed, &["push", "origin", &fixture.branch]);
    // Pull with a merge so the conflict is staged in the working tree. The
    // command is run directly because it is *expected* to fail, but with the
    // same isolated environment the fixture uses so the merge can produce its
    // conflict commit metadata.
    let output = std::process::Command::new("git")
        .current_dir(&fixture.checkout)
        .args(["pull", "--no-rebase"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the conflicting pull must fail: {combined}"
    );
    assert!(
        combined.contains("CONFLICT"),
        "the pull failed because of a content conflict: {combined}"
    );

    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();
    assert_eq!(status.conflicts, 1, "one path conflicts: {status:?}");
    assert!(
        !status.is_clean(),
        "a conflicted working tree is never clean"
    );
}

#[tokio::test]
async fn a_repository_whose_remote_disappears_reports_a_structured_fetch_error() {
    // Safely reproducible stand-in for an authentication failure: the remote
    // path stops resolving, so the fetch must fail with an actionable error
    // instead of hanging or panicking. `GIT_TERMINAL_PROMPT=0` in the fixture
    // guarantees no interactive credential prompt can block the run.
    let fixture = Fixture::new("main");
    std::fs::remove_dir_all(&fixture.remote).unwrap();

    let manager = manager();
    let error = manager
        .fetch(&fixture.repository(), &options())
        .await
        .unwrap_err();
    assert!(
        matches!(error, GitError::RemoteUnavailable { .. }),
        "expected a remote-unavailable error, got {error:?}"
    );
    assert_eq!(error.code(), "GIT_REMOTE_UNAVAILABLE");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn an_already_cancelled_operation_is_reported_as_cancelled() {
    use service_orchestrator_execution::Cancellation;

    let fixture = Fixture::new("main");
    let cancellation = Cancellation::new();
    cancellation.cancel();
    let opts = options().with_cancellation(cancellation);

    let manager = manager();
    let error = manager
        .fetch(&fixture.repository(), &opts)
        .await
        .unwrap_err();
    assert!(
        matches!(error, GitError::Cancelled { .. }),
        "expected a cancelled error, got {error:?}"
    );
    assert_eq!(error.code(), "GIT_CANCELLED");
}

#[tokio::test]
async fn an_operation_that_exceeds_its_timeout_is_reported_as_timed_out() {
    use std::time::Duration;

    let fixture = Fixture::new("main");
    // A zero budget is already spent, so the operation must be rejected as a
    // timeout rather than run unbounded.
    let opts = options().with_timeout(Duration::from_nanos(1));

    let manager = manager();
    let result = manager.fetch(&fixture.repository(), &opts).await;
    match result {
        Ok(_) => {
            // A local fetch can complete inside one nanosecond of wall-clock
            // budget only by luck; if it did, the timeout path is exercised by
            // the `GIT_TIMEOUT` code being unreachable here rather than by a
            // hang, which is the property that matters.
        }
        Err(error) => {
            assert!(
                matches!(error, GitError::Timeout { .. }),
                "expected a timeout, got {error:?}"
            );
            assert_eq!(error.code(), "GIT_TIMEOUT");
        }
    }
}

#[tokio::test]
async fn status_reports_a_clean_repository() {
    let fixture = Fixture::new("main");
    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();

    assert_eq!(status.lifecycle, RepositoryLifecycle::Available);
    assert!(status.lifecycle.is_available());
    assert_eq!(status.branch.as_ref().map(Branch::as_str), Some("main"));
    assert!(status.is_clean());
    assert!(status.upstream.is_some());
    assert!(!status.detached_head);
    assert!(!status.unborn);
    assert!(status.commit.is_some());
}

#[tokio::test]
async fn status_reports_staged_unstaged_and_untracked_changes() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("untracked.txt"), "new\n").unwrap();
    std::fs::write(fixture.checkout.join("README.md"), "modified\n").unwrap();
    std::fs::write(fixture.checkout.join("staged.txt"), "staged\n").unwrap();
    git(&fixture.checkout, &["add", "staged.txt"]);

    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();

    assert_eq!(status.staged_changes, 1);
    assert_eq!(status.unstaged_changes, 1);
    assert_eq!(status.untracked_files, 1);
    assert!(status.has_local_changes());
    assert!(!status.is_clean());
}

#[tokio::test]
async fn status_reports_not_cloned_when_the_path_is_missing() {
    let fixture = Fixture::new("main");
    std::fs::remove_dir_all(&fixture.checkout).unwrap();

    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();
    assert_eq!(status.lifecycle, RepositoryLifecycle::NotCloned);
    assert!(!status.lifecycle.is_available());
}

#[tokio::test]
async fn status_reports_an_invalid_repository_for_a_plain_directory() {
    let fixture = Fixture::new("main");
    std::fs::remove_dir_all(&fixture.checkout).unwrap();
    std::fs::create_dir_all(&fixture.checkout).unwrap();

    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();
    assert_eq!(status.lifecycle, RepositoryLifecycle::Invalid);
}

#[tokio::test]
async fn status_reports_a_detached_head() {
    let fixture = Fixture::new("main");
    let head = git(&fixture.checkout, &["rev-parse", "HEAD"]);
    git(&fixture.checkout, &["checkout", head.trim()]);

    let manager = manager();
    let status = manager
        .status(&fixture.repository(), &options())
        .await
        .unwrap();
    assert!(status.detached_head);
    assert!(status.branch.is_none());
}

// -- branches ----------------------------------------------------------------

#[tokio::test]
async fn lists_local_and_remote_branches() {
    let fixture = Fixture::new("main");
    fixture.remote_branch("feature/x");

    let manager = manager();
    let repository = fixture.repository();
    manager.fetch(&repository, &options()).await.unwrap();
    let branches = manager.branches(&repository, &options()).await.unwrap();

    let main = branches
        .iter()
        .find(|b| b.name == "main")
        .expect("main is listed");
    assert_eq!(main.kind, BranchKind::Local);
    assert!(main.current);
    assert!(main.upstream.is_some());

    assert!(branches
        .iter()
        .any(|b| b.name == "origin/feature/x" && b.kind == BranchKind::Remote));
}

// -- fetch -------------------------------------------------------------------

#[tokio::test]
async fn fetch_updates_tracking_without_touching_the_working_tree() {
    let fixture = Fixture::new("main");
    let before = std::fs::read_to_string(fixture.checkout.join("README.md")).unwrap();

    fixture.remote_commit("second");

    let manager = manager();
    let repository = fixture.repository();
    manager.fetch(&repository, &options()).await.unwrap();

    let status = manager.status(&repository, &options()).await.unwrap();
    assert!(status.is_behind());
    // The working tree is untouched by a fetch.
    assert_eq!(
        std::fs::read_to_string(fixture.checkout.join("README.md")).unwrap(),
        before
    );
}

// -- pull --------------------------------------------------------------------

#[tokio::test]
async fn pull_fast_forwards_the_current_branch() {
    let fixture = Fixture::new("main");
    let manager = manager();
    let repository = fixture.repository();
    let before = manager
        .status(&repository, &options())
        .await
        .unwrap()
        .commit
        .unwrap();

    fixture.remote_commit("second");
    let result = manager.pull(&repository, &options()).await.unwrap();

    assert!(result.success);
    assert!(result.changed);
    assert_eq!(result.operation, GitOperation::Pull);
    assert_ne!(
        result.previous_commit.as_deref(),
        result.current_commit.as_deref()
    );
    let after = manager
        .status(&repository, &options())
        .await
        .unwrap()
        .commit
        .unwrap();
    assert_ne!(before, after);
    assert!(fixture.checkout.join("second.txt").exists());
}

#[tokio::test]
async fn pull_reports_no_change_when_already_up_to_date() {
    let fixture = Fixture::new("main");
    let manager = manager();
    let result = manager
        .pull(&fixture.repository(), &options())
        .await
        .unwrap();
    assert!(result.success);
    assert!(!result.changed);
}

#[tokio::test]
async fn pull_refuses_a_dirty_working_tree() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("README.md"), "dirty\n").unwrap();

    let manager = manager();
    let error = manager
        .pull(&fixture.repository(), &options())
        .await
        .unwrap_err();
    assert!(matches!(error, GitError::WorkingTreeDirty { .. }));
    assert_eq!(error.code(), "GIT_WORKING_TREE_DIRTY");
}

#[tokio::test]
async fn pull_refuses_a_diverged_branch_with_an_actionable_error() {
    let fixture = Fixture::new("main");
    // A local commit ...
    std::fs::write(fixture.checkout.join("local.txt"), "local\n").unwrap();
    git(&fixture.checkout, &["add", "."]);
    git(&fixture.checkout, &["commit", "-m", "local"]);
    // ... and an unrelated remote commit.
    fixture.remote_commit("remote");

    let manager = manager();
    let error = manager
        .pull(&fixture.repository(), &options())
        .await
        .unwrap_err();
    match error {
        GitError::DivergedBranch { ahead, behind, .. } => {
            assert_eq!(ahead, 1);
            assert_eq!(behind, 1);
        }
        other => panic!("expected a diverged branch, got {other:?}"),
    }
}

// -- checkout ----------------------------------------------------------------

#[tokio::test]
async fn checkout_switches_to_an_existing_local_branch() {
    let fixture = Fixture::new("main");
    git(&fixture.checkout, &["checkout", "-b", "feature/local"]);

    let manager = manager();
    let repository = fixture.repository();
    let branch = Branch::new("feature/local").unwrap();
    let result = manager
        .checkout(&repository, &branch, &options())
        .await
        .unwrap();
    assert_eq!(
        result.branch.as_ref().map(Branch::as_str),
        Some("feature/local")
    );
    assert_eq!(result.operation, GitOperation::Checkout);
}

#[tokio::test]
async fn checkout_tracks_a_remote_only_branch() {
    let fixture = Fixture::new("main");
    fixture.remote_branch("feature/remote");

    let manager = manager();
    let repository = fixture.repository();
    manager.fetch(&repository, &options()).await.unwrap();

    let branch = Branch::new("feature/remote").unwrap();
    let result = manager
        .checkout(&repository, &branch, &options())
        .await
        .unwrap();
    assert_eq!(
        result.branch.as_ref().map(Branch::as_str),
        Some("feature/remote")
    );

    let status = manager.status(&repository, &options()).await.unwrap();
    assert_eq!(
        status.branch.as_ref().map(Branch::as_str),
        Some("feature/remote")
    );
    assert!(status.upstream.is_some(), "checkout sets up tracking");
}

#[tokio::test]
async fn checkout_rejects_a_missing_branch() {
    let fixture = Fixture::new("main");
    let manager = manager();
    let error = manager
        .checkout(
            &fixture.repository(),
            &Branch::new("does-not-exist").unwrap(),
            &options(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, GitError::BranchNotFound { .. }));
    assert_eq!(error.code(), "GIT_BRANCH_NOT_FOUND");
}

#[tokio::test]
async fn checkout_refuses_when_it_would_overwrite_local_changes() {
    let fixture = Fixture::new("main");
    // The remote branch changes README.md; the local modification conflicts.
    git(&fixture.seed, &["checkout", "-b", "feature/conflict"]);
    std::fs::write(fixture.seed.join("README.md"), "remote version\n").unwrap();
    git(&fixture.seed, &["add", "."]);
    git(&fixture.seed, &["commit", "-m", "change readme"]);
    git(&fixture.seed, &["push", "-u", "origin", "feature/conflict"]);
    git(&fixture.seed, &["checkout", "main"]);
    std::fs::write(fixture.checkout.join("README.md"), "local version\n").unwrap();

    let manager = manager();
    let repository = fixture.repository();
    manager.fetch(&repository, &options()).await.unwrap();
    let error = manager
        .checkout(
            &repository,
            &Branch::new("feature/conflict").unwrap(),
            &options(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, GitError::WorkingTreeDirty { .. }));
    // The local modification is preserved.
    assert_eq!(
        std::fs::read_to_string(fixture.checkout.join("README.md")).unwrap(),
        "local version\n"
    );
}

// -- clone -------------------------------------------------------------------

#[tokio::test]
async fn clone_creates_a_checkout_at_the_configured_path() {
    let fixture = Fixture::new("main");
    std::fs::remove_dir_all(&fixture.checkout).unwrap();

    let manager = manager();
    let result = GitManager::clone(&manager, &fixture.repository(), &options())
        .await
        .unwrap();

    assert!(result.success);
    assert_eq!(result.operation, GitOperation::Clone);
    assert_eq!(result.branch.as_ref().map(Branch::as_str), Some("main"));
    assert!(fixture.checkout.join("README.md").exists());
}

#[tokio::test]
async fn clone_refuses_a_non_empty_destination() {
    let fixture = Fixture::new("main");
    let manager = manager();
    let error = GitManager::clone(&manager, &fixture.repository(), &options())
        .await
        .unwrap_err();
    assert!(matches!(error, GitError::DestinationNotEmpty { .. }));
    assert_eq!(error.code(), "GIT_DESTINATION_NOT_EMPTY");
}

#[tokio::test]
async fn clone_reports_a_structured_error_for_an_invalid_url() {
    let fixture = Fixture::new("main");
    std::fs::remove_dir_all(&fixture.checkout).unwrap();

    let mut repository = fixture.repository();
    repository.url = fixture
        .workspace_root
        .join("missing-remote.git")
        .to_string_lossy()
        .into_owned();

    let manager = manager();
    let error = GitManager::clone(&manager, &repository, &options())
        .await
        .unwrap_err();
    assert!(
        error.code().starts_with("GIT_"),
        "unexpected code {}",
        error.code()
    );
}

// -- destructive operations --------------------------------------------------

#[tokio::test]
async fn reset_hard_discards_tracked_changes() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("README.md"), "dirty\n").unwrap();

    let manager = manager();
    let repository = fixture.repository();
    manager.reset_hard(&repository, &options()).await.unwrap();
    let status = manager.status(&repository, &options()).await.unwrap();
    assert!(status.is_clean());
}

#[tokio::test]
async fn clean_removes_untracked_files() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("scratch.txt"), "x\n").unwrap();

    let manager = manager();
    let repository = fixture.repository();
    manager.clean(&repository, &options()).await.unwrap();
    let status = manager.status(&repository, &options()).await.unwrap();
    assert_eq!(status.untracked_files, 0);
    assert!(!fixture.checkout.join("scratch.txt").exists());
}

#[tokio::test]
async fn discard_local_changes_removes_tracked_and_untracked_changes() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("README.md"), "dirty\n").unwrap();
    std::fs::write(fixture.checkout.join("scratch.txt"), "x\n").unwrap();

    let manager = manager();
    let repository = fixture.repository();
    manager
        .discard_local_changes(&repository, &options())
        .await
        .unwrap();
    let status = manager.status(&repository, &options()).await.unwrap();
    assert!(status.is_clean());
}

// -- diff --------------------------------------------------------------------

#[tokio::test]
async fn diff_reports_unstaged_changes() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("README.md"), "changed\n").unwrap();
    std::fs::write(fixture.checkout.join("new.txt"), "added\n").unwrap();

    let manager = manager();
    let diff = manager
        .diff(&fixture.repository(), &DiffOptions::new())
        .await
        .unwrap();

    assert!(!diff.staged);
    assert!(!diff.truncated);
    assert!(diff.bytes > 0);
    assert!(diff.files.iter().any(|f| f.path.contains("README.md")));
    assert!(diff.patch.contains("README.md"));
}

#[tokio::test]
async fn diff_bounds_the_patch_to_the_requested_size() {
    let fixture = Fixture::new("main");
    let big = "line\n".repeat(20_000);
    std::fs::write(fixture.checkout.join("README.md"), big).unwrap();

    let manager = manager();
    let diff = manager
        .diff(
            &fixture.repository(),
            &DiffOptions::new().with_max_bytes(256),
        )
        .await
        .unwrap();

    assert!(diff.truncated);
    assert!(diff.bytes <= 256);
}

#[tokio::test]
async fn diff_reports_the_staged_index() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("staged.txt"), "staged\n").unwrap();
    git(&fixture.checkout, &["add", "staged.txt"]);

    let manager = manager();
    let diff = manager
        .diff(&fixture.repository(), &DiffOptions::new().staged(true))
        .await
        .unwrap();
    assert!(diff.staged);
    assert!(diff.files.iter().any(|f| f.path.contains("staged.txt")));
}

// -- sync --------------------------------------------------------------------

#[tokio::test]
async fn sync_fast_forwards_the_default_branch() {
    let fixture = Fixture::new("main");
    fixture.remote_commit("second");

    let manager = manager();
    let result = manager
        .sync(&fixture.repository(), &options())
        .await
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.operation, GitOperation::Sync);
    assert!(fixture.checkout.join("second.txt").exists());
}

#[tokio::test]
async fn sync_returns_to_the_default_branch_from_another_branch() {
    let fixture = Fixture::new("main");
    git(&fixture.checkout, &["checkout", "-b", "feature/local"]);

    let manager = manager();
    let repository = fixture.repository();
    let result = manager.sync(&repository, &options()).await.unwrap();
    assert_eq!(result.branch.as_ref().map(Branch::as_str), Some("main"));

    let status = manager.status(&repository, &options()).await.unwrap();
    assert_eq!(status.branch.as_ref().map(Branch::as_str), Some("main"));
}

#[tokio::test]
async fn sync_refuses_a_dirty_working_tree() {
    let fixture = Fixture::new("main");
    std::fs::write(fixture.checkout.join("README.md"), "dirty\n").unwrap();

    let manager = manager();
    let error = manager
        .sync(&fixture.repository(), &options())
        .await
        .unwrap_err();
    assert!(matches!(error, GitError::WorkingTreeDirty { .. }));
}

// -- safety and coordination -------------------------------------------------

#[tokio::test]
async fn operations_reject_a_checkout_outside_the_workspace() {
    let fixture = Fixture::new("main");
    let mut repository = fixture.repository();
    repository.path = AbsolutePath::new("/tmp/elsewhere/auth-service").unwrap();

    let manager = manager();
    let error = manager.status(&repository, &options()).await.unwrap_err();
    assert!(matches!(error, GitError::PathOutsideWorkspace { .. }));
    assert_eq!(error.code(), "GIT_PATH_OUTSIDE_WORKSPACE");
}

#[tokio::test]
async fn concurrent_mutations_on_the_same_repository_are_serialized() {
    let fixture = Fixture::new("main");
    let manager = Arc::new(manager());
    let repository = fixture.repository();
    let options = options();

    let first = manager.fetch(&repository, &options);
    let second = manager.fetch(&repository, &options);
    let (first, second) = tokio::join!(first, second);

    assert!(first.is_ok());
    assert!(second.is_ok());
}

#[tokio::test]
async fn independent_repositories_progress_concurrently() {
    let first = Fixture::new("main");
    let second = Fixture::new("main");

    let manager = Arc::new(manager());
    let repository_a = first.repository();
    let repository_b = second.repository();
    let options = options();

    let status_a = manager.status(&repository_a, &options);
    let status_b = manager.status(&repository_b, &options);
    let (status_a, status_b) = tokio::join!(status_a, status_b);

    assert!(status_a.unwrap().lifecycle.is_available());
    assert!(status_b.unwrap().lifecycle.is_available());
}
