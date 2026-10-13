//! Read-only application queries.
//!
//! Queries are the "Q" half of the commands/queries split. They are side-effect
//! free and return structured application values. Separating them from commands
//! keeps the read path predictable: a status query cannot accidentally fetch a
//! repository or start a process.

mod build;
mod configuration;
mod git;
mod health;
mod library;
mod migration;
mod repository;
mod runtime;
mod service;
mod task;
mod validation;
mod workspace;
