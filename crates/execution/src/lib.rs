//! # Execution
//!
//! Task and workflow engine.
//!
//! Owns task state, dependency-aware scheduling, bounded concurrency,
//! cancellation, retry, timeout, and progress reporting. It depends only on
//! `service-orchestrator-domain` and never on interfaces or concrete
//! infrastructure. Concrete task handlers are supplied by outer layers.
