//! Port-conflict detection.
//!
//! Before starting a service, the manager checks each configured port and
//! reports a conflict instead of starting into it (`tasks/010.md` §21). Detection
//! is deliberately conservative: it opens a TCP connection to localhost and
//! treats a *successful* connection as "in use". Anything else — including a
//! timeout on a filtered port — is treated as free, so the check never blocks a
//! legitimate start on a false positive. The manager never kills or reclaims the
//! occupying process.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// Answers whether a local port is already in use.
pub trait PortChecker: Send + Sync {
    /// Returns `true` when something is accepting connections on `port`.
    fn is_in_use(&self, port: u16) -> bool;
}

/// The real [`PortChecker`], probing localhost over TCP.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPortChecker {
    timeout: Duration,
}

impl SystemPortChecker {
    /// Creates a checker with a conservative default timeout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_millis(250),
        }
    }

    /// Overrides the connection timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl PortChecker for SystemPortChecker {
    fn is_in_use(&self, port: u16) -> bool {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        TcpStream::connect_timeout(&address, self.timeout).is_ok()
    }
}

/// A [`PortChecker`] that never reports a conflict.
///
/// Used by tests and by compositions that deliberately disable port checks. It
/// is intentionally named for what it does rather than hidden as a default:
/// production wiring chooses [`SystemPortChecker`].
#[derive(Clone, Copy, Debug, Default)]
pub struct FreePortChecker;

impl PortChecker for FreePortChecker {
    fn is_in_use(&self, _port: u16) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_port_checker_never_reports_a_conflict() {
        assert!(!FreePortChecker.is_in_use(8081));
    }

    #[test]
    fn a_bound_listener_is_detected() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let checker = SystemPortChecker::new();
        assert!(checker.is_in_use(port));
    }

    #[test]
    fn an_unbound_port_is_free() {
        // Bind and immediately drop to obtain a very likely free port.
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let checker = SystemPortChecker::new();
        assert!(!checker.is_in_use(port));
    }
}
