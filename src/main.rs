//! Service Orchestrator binary — the composition root.
//!
//! This crate is intentionally thin. Its only responsibility is to wire
//! concrete infrastructure implementations into the application core and then
//! dispatch to an interface adapter (`tui`, `mcp`, or a future `cli`/`api`).
//!
//! No business logic belongs here. See `docs/architecture.md` for the full
//! architecture and `docs/adr/001-core-architecture.md` for the rationale.

fn main() {
    // Interface dispatch (`tui`, `mcp`, `doctor`) is established in follow-up
    // work. The foundation intentionally stops at crate boundaries and wiring.
    println!("service-orchestrator: foundation only, no interfaces implemented yet");
}
