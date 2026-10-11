//! Reusable presentation widgets shared by the screens.
//!
//! Nothing here reaches application state; each widget receives already-read
//! values. This keeps the visual system in one place and the screens thin.

pub(crate) mod dialog;
pub(crate) mod footer;
pub(crate) mod status;
pub(crate) mod table;
