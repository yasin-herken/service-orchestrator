//! # Health
//!
//! Infrastructure adapter for health checks.
//!
//! Supports configurable checks such as process, port, and HTTP/actuator
//! probes. A live process does not imply a healthy service. It depends on the
//! domain and the application core.
