# Task 023 progress — Workspace & Service Configuration Management

Status: **complete**.

## Plan (as executed)

1. `config` crate: exposed the raw TOML schema (`pub raw`), added a
   format-preserving `ConfigRepository`/`FileConfigManager` with
   create/update/remove, validation, diff/preview, revision-based concurrency,
   and atomic persistence (`crates/config/src/management.rs`).
2. `application` crate: configuration use cases delegated to the `ConfigManager`
   port; a read-only `RepositoryInspector` port for inspection + technology
   detection; `Application::with_configuration` / `with_repository_inspector`.
3. `daemon` crate: owns the live configuration, refreshes the application after
   a commit/reload, broadcasts `ConfigurationChanged`, and routes the new
   operations under the existing policy.
4. `client` crate: new ops, the `ConfigurationChanged` protocol event, trait
   defaults, and the `RemoteClient` demux.
5. Desktop Tauri adapter: a thin `configuration` command module, command
   registration, the `daemon://configuration` event forwarder, and error-code
   promotion.
6. React: the **Configuration** screen (workspaces, service wizard, libraries,
   profiles, groups, validate/preview/diff/save, repository inspection).
7. Tests (Rust unit + integration + frontend) and documentation.

## Decisions

- The daemon is the only writer of `config.toml`; Tauri and React never touch
  the file. React builds a typed `ConfigChange`, previews it, and commits it.
- Comment/format preservation uses `toml_edit`; only the edited array of tables
  is rewritten (rewritten nested entities render as inline tables).
- Revision-based concurrency: a stale revision fails with `CONFIG_CONFLICT` and
  writes nothing; the TOCTOU window is closed by re-checking the revision
  immediately before the atomic rename.
- Secrets are redacted in diffs/logs and written back verbatim when unchanged;
  the GUI masks a secret value until explicitly revealed.
- Gradle is detected but reported as `provider_not_installed` — never as
  operational. No Gradle provider is implemented in this task.

## Verification (actually run)

- Rust workspace: `cargo fmt --check`, `cargo check --workspace`,
  `cargo test --workspace`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` — all clean.
- Tauri crate (its own workspace): `cargo check --all-targets` and
  `cargo clippy --all-targets` — clean; 40 integration tests green.
- Frontend: `npm run typecheck`, `npm test` (202 tests), `npm run build` —
  all green.
