//! # MCP
//!
//! MCP interface adapter (stdio transport).
//!
//! A thin adapter that validates input, applies policy, and forwards calls to
//! application use cases. It must not implement business logic or call
//! infrastructure directly. It depends only on the application core.
