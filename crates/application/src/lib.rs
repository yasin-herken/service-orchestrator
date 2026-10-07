//! # Application
//!
//! Use cases, orchestration, and infrastructure port traits.
//!
//! This is the shared core used by every interface. It coordinates the domain
//! and the execution engine and declares the ports (traits) that
//! infrastructure crates implement, such as Git, process, build, liquibase,
//! health, and logging.
//!
//! It depends on the domain, the execution engine, and the policy engine. It
//! must never depend on `tui`, `mcp`, or concrete infrastructure crates.
