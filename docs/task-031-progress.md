# Task 031 — Progress Record

Working notes for **macOS App Packaging, Signing, Notarization & DMG
Distribution**. This is a context-recovery record, not user documentation; the
results are in [`packaging-report.md`](packaging-report.md) and the guide is
[`macos-distribution.md`](macos-distribution.md).

## Decisions

- **Bundle the orchestration binary as a Tauri external binary (sidecar).**
  `tauri.conf.json` sets `bundle.externalBin = ["binaries/service-orchestrator"]`.
  Tauri copies it to `Contents/MacOS/service-orchestrator` and signs it with the
  rest of the bundle. The GUI resolves it as a sibling of its own executable
  first, then `<bundle>/Contents/Resources`, then `PATH`, then the development
  `target/` layout (`apps/desktop/src-tauri/src/daemon_bin.rs`).
- **A bundled sidecar wins over `PATH`.** A packaged application must be
  self-contained; a `service-orchestrator` the user happens to have installed
  elsewhere must not silently replace the shipped one. `PATH` remains the
  fallback for the development layout.
- **No shell plugin.** The GUI still has `core:default` only. Bundling a sidecar
  does not require the shell plugin and does not widen the webview's permissions.
- **Safe static `PATH` discovery.** `crates/client/src/path_env.rs` augments the
  daemon's `PATH` with existing developer-tool directories (Homebrew, SDKMAN,
  nvm, fnm, Volta, `JAVA_HOME/bin`) when the GUI auto-starts it. No login shell
  is ever executed.
- **Minimal entitlements.** `apps/desktop/src-tauri/entitlements.plist` grants
  only the two keys the WKWebView JavaScript engine needs under the hardened
  runtime (`allow-jit`, `allow-unsigned-executable-memory`).
- **Original placeholder icon.** Generated from `apps/desktop/icon-source.png`
  by `apps/desktop/scripts/generate-icon.py`; branding remains provisional.
- **Signing is opt-in; the bundle is always sealed.** `APPLE_SIGNING_IDENTITY`
  is read from the standard Tauri environment and overrides the default; with no
  identity the build ad-hoc signs (identity `-`) so the bundle is internally
  consistent (`codesign --verify --deep --strict` passes, Gatekeeper shows the
  normal "unidentified developer" prompt instead of reporting a damaged app).
  Notarization stays opt-in and needs Apple credentials. Nothing is stored in
  the repository.

## Prerequisite change

`tauri-build` validates `bundle.externalBin` while compiling the desktop crate,
so the triple-named sidecar must exist before `cargo test`/`tauri dev`/`tauri
build` touch that crate. `apps/desktop/scripts/preflight.mjs` now builds the
orchestration binary **and stages the sidecar**; `scripts/test-e2e.sh` calls it
with `--stage-only` before the desktop tests.

## Environment limits

- Apple Silicon only (`aarch64-apple-darwin`); the machine had no `rustup` and
  no `x86_64-apple-darwin` standard library, so Intel and universal builds could
  not be produced or verified here.
- No Apple Developer credentials, so the reference build is ad-hoc signed and
  unnotarized.

## Status

- [x] Tauri bundle configuration, identity, icons, entitlements.
- [x] Sidecar bundling and bundle-aware resolution + tests.
- [x] Static `PATH` discovery + tests.
- [x] `scripts/build-macos.sh`, `scripts/verify-macos-bundle.sh`.
- [x] `docs/macos-distribution.md`, README, docs index, known limitations.
- [x] `.app` + `.dmg` built, ad-hoc sealed, and verified.
- [x] Packaged-daemon runtime verified: clean-environment start, socket/pid/log
      permissions, IPC status, clean stop.
- [x] GUI first launch verified: bundled-sibling resolution, auto-start, `PATH`
      augmentation, daemon survival after the GUI closes.
- [x] DMG install verified: mount, copy, signature, run from the installed copy.
- [x] `docs/packaging-report.md` written from measured results.
- [x] `--debug` build option fixed (it now passes `--debug` to `tauri build`
      and resolves the `debug/` bundle directory; previously it looked for a
      debug bundle that was never produced).
