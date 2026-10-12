//! The real, read-only workspace probe.
//!
//! The application core never touches the filesystem, `PATH`, or the network
//! itself; it observes the environment only through the
//! [`WorkspaceProbe`] port. This module is the composition root's
//! implementation of that port: existence checks against `std::fs`, program
//! discovery against `PATH`, and port occupancy against a loopback TCP connect
//! probe (`tasks/017.md` §7).
//!
//! Every operation here is side-effect free. Nothing in this file creates,
//! writes, moves, or deletes anything.

use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use service_orchestrator_application::WorkspaceProbe;

/// How long a single port probe may take before the port is deemed free.
const PORT_PROBE_TIMEOUT: Duration = Duration::from_millis(150);

/// A read-only probe of the local machine.
///
/// Constructed once in the composition root and handed to the application core
/// through [`Application::with_workspace_probe`].
///
/// [`Application::with_workspace_probe`]: service_orchestrator_application::Application::with_workspace_probe
/// [`WorkspaceProbe`]: service_orchestrator_application::WorkspaceProbe
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemWorkspaceProbe;

impl WorkspaceProbe for SystemWorkspaceProbe {
    fn exists(&self, path: &str) -> bool {
        Path::new(path).symlink_metadata().is_ok()
    }

    fn is_dir(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }

    fn contains_entry(&self, path: &str, entry: &str) -> bool {
        Path::new(path).join(entry).symlink_metadata().is_ok()
    }

    fn has_program(&self, program: &str) -> bool {
        if program.contains('/') {
            // An explicit path is checked directly rather than searched.
            return is_executable(Path::new(program));
        }
        let Some(paths) = env::var_os("PATH") else {
            return false;
        };
        env::split_paths(&paths).any(|directory| is_executable(&directory.join(program)))
    }

    fn is_port_in_use(&self, port: u16) -> bool {
        let mut addresses = match ("127.0.0.1", port).to_socket_addrs() {
            Ok(addresses) => addresses,
            Err(_) => return false,
        };
        addresses.any(|address| TcpStream::connect_timeout(&address, PORT_PROBE_TIMEOUT).is_ok())
    }
}

/// Returns `true` when `path` is an existing executable file (or a symlink to
/// one) for the current user.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match path.metadata() {
            // `symlink_metadata` would reject symlinks to real binaries, which
            // is common for version managers, so follow the link here.
            Ok(metadata) => metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}
