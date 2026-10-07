//! # Config
//!
//! Configuration loading, parsing, validation, versioning, and migration.
//!
//! Configuration is external and versioned; services are never hard-coded in
//! source. This crate is infrastructure: it produces domain configuration
//! values and depends only on the domain. It performs no orchestration.
