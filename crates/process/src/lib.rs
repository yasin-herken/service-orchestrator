//! # Process
//!
//! Infrastructure adapter for local process management.
//!
//! Tracks PIDs, commands, states, exit codes, ports, child processes, and logs,
//! and performs graceful and forced termination. It depends on the domain and
//! the application core and is wired in by the composition root.
