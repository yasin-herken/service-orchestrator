//! Read-only environment probing for workspace validation.
//!
//! Workspace validation must never modify anything (`tasks/017.md` §7). The
//! application core therefore reaches the filesystem and the network only
//! through this boundary, which exposes observations — never mutations.

/// A read-only probe of the local environment.
///
/// Implementations must be side-effect free: existence checks, program
/// lookups, and connect probes only.
pub trait WorkspaceProbe: Send + Sync {
    /// Returns `true` when `path` exists (following symlinks).
    fn exists(&self, path: &str) -> bool;

    /// Returns `true` when `path` exists and is a directory.
    fn is_dir(&self, path: &str) -> bool;

    /// Returns `true` when the directory `path` contains an entry named
    /// `entry` (for example `.git`).
    fn contains_entry(&self, path: &str, entry: &str) -> bool;

    /// Returns `true` when `program` is resolvable on the `PATH`.
    fn has_program(&self, program: &str) -> bool;

    /// Returns `true` when something is currently accepting TCP connections on
    /// the loopback `port`.
    fn is_port_in_use(&self, port: u16) -> bool;
}

/// A permissive probe used when no real probe is wired.
///
/// It assumes every path, directory, and program exists and no port is
/// occupied, so validation reports no environment findings. Production
/// wiring always replaces it with a real probe in the composition root;
/// it exists so [`Application::new`](crate::Application::new) needs no
/// environment access of its own.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NullWorkspaceProbe;

impl WorkspaceProbe for NullWorkspaceProbe {
    fn exists(&self, _path: &str) -> bool {
        true
    }

    fn is_dir(&self, _path: &str) -> bool {
        true
    }

    fn contains_entry(&self, _path: &str, _entry: &str) -> bool {
        true
    }

    fn has_program(&self, _program: &str) -> bool {
        true
    }

    fn is_port_in_use(&self, _port: u16) -> bool {
        false
    }
}
