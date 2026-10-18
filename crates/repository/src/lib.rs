//! # Repository
//!
//! The system repository adapter: the read-only filesystem implementations of
//! the application's repository boundaries.
//!
//! ```text
//! Application use case
//!        │  RepositoryInspector port        │  RepositoryDiscovery port
//!        ▼                                  ▼
//! SystemRepositoryInspector          SystemRepositoryDiscovery
//!        │  (read well-known files)         │  (bounded walk + inspector)
//!        ▼                                  ▼
//!      Filesystem                        Filesystem
//! ```
//!
//! Both adapters are **reads only**: the inspector reads well-known files (and
//! at most a small prefix of any one of them), and the walker bounds its
//! traversal by depth, exclusion list, symlink policy, and a repository cap.
//! Neither runs a build, a script, `npm install`, or a Git command, and neither
//! writes (`tasks/023.md` §17, §20; `tasks/032.md` §7, §8).
//!
//! Everything they report is an *observation* — a suggestion the interfaces
//! present for review, never a silently committed fact.
//!
//! See `docs/service-templates.md`, `docs/bulk-service-import.md`, and
//! `docs/adr/023-service-wizard.md`.

mod discovery;
mod inspector;

pub use discovery::SystemRepositoryDiscovery;
pub use inspector::SystemRepositoryInspector;
