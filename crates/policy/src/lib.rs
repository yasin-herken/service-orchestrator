//! # Policy
//!
//! Permission model and policy engine.
//!
//! Classifies operations (for example `READ`, `SAFE_WRITE`, `DESTRUCTIVE`) and
//! decides whether an operation may proceed. Every interface, including MCP,
//! must pass through this engine and must not bypass it. Depends only on the
//! domain.
