# Packaging Report — Task 031

This is the measured result of the reference macOS build produced for **Task
031 — macOS App Packaging, Signing, Notarization & DMG Distribution**. Every
number below was read from the artifacts in this working tree, not estimated.

The operational guide is [`macos-distribution.md`](macos-distribution.md); the
context-recovery notes are [`task-031-progress.md`](task-031-progress.md).

---

## 1. Build environment

| Item | Value |
| --- | --- |
| macOS | 26.6.2 (build 25G83), arm64 |
| Rust | `cargo 1.98.1 (797e8a9bc 2026-08-05)`, `rustc 1.98.1 (48a229cea 2026-09-01)` |
| Rust host target | `aarch64-apple-darwin` |
| Node.js / npm | v22.18.0 / 11.11.1 |
| Tauri CLI | 2.12.2 (`@tauri-apps/cli`) |
| Xcode Command Line Tools | present (`codesign`, `lipo`, `hdiutil`, `file`) |
| Apple signing credentials | **none present** |

The machine had only the `aarch64-apple-darwin` Rust standard library and no
`rustup` command, so no Intel or universal build could be produced here (see
§14).

---

## 2. Application version

One authoritative version, **0.1.0**, agrees across the Rust workspace
(`Cargo.toml`), the Tauri config (`apps/desktop/src-tauri/tauri.conf.json`), the
bundle `Info.plist` (`CFBundleShortVersionString`/`CFBundleVersion`), and the
DMG filename. `scripts/build-macos.sh` refuses to build if the first two
disagree.

---

## 3. Generated artifacts

| Artifact | Path (from the repository root) | Size |
| --- | --- | --- |
| Application bundle | `apps/desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Service Orchestrator.app` | **32.50 MiB** (33 MiB on disk) |
| Disk image | `apps/desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/Service Orchestrator_0.1.0_aarch64.dmg` | **8.96 MiB** (9,396,525 bytes) |

### Bundle structure

```text
Service Orchestrator.app
└── Contents
    ├── Info.plist
    ├── MacOS
    │   ├── service-orchestrator-desktop   18,612,144 bytes  Mach-O arm64
    │   └── service-orchestrator           15,231,968 bytes  Mach-O arm64
    ├── Resources
    │   └── icon.icns
    └── _CodeSignature
        └── CodeResources                   2,670 bytes
```

The frontend is compiled into the GUI executable (Tauri's default
`frontendDist` embedding); there is no loose `dist/` directory in the bundle.
The orchestration binary is the Tauri `externalBin` sidecar
(`bundle.externalBin = ["binaries/service-orchestrator"]`), placed by Tauri at
`Contents/MacOS/service-orchestrator`, next to the GUI.

### Bundle metadata (`Contents/Info.plist`)

| Key | Value |
| --- | --- |
| `CFBundleName` / `CFBundleDisplayName` | Service Orchestrator |
| `CFBundleIdentifier` | `com.service-orchestrator.desktop` |
| `CFBundleShortVersionString` / `CFBundleVersion` | 0.1.0 |
| `CFBundleExecutable` | `service-orchestrator-desktop` |
| `CFBundleIconFile` | `icon.icns` |
| `LSMinimumSystemVersion` | 11.0 |
| `LSApplicationCategoryType` | `public.app-category.developer-tools` |
| `NSHighResolutionCapable` | true |
| `NSHumanReadableCopyright` | Copyright © 2026 Service Orchestrator contributors. Licensed under Apache-2.0. |

---

## 4. Architecture verification

Both executables in the bundle are native Apple Silicon and were checked
directly:

```text
$ lipo -archs "…/Contents/MacOS/service-orchestrator-desktop"
arm64
$ lipo -archs "…/Contents/MacOS/service-orchestrator"
arm64
$ file "…/Contents/MacOS/service-orchestrator"
Mach-O 64-bit executable arm64
```

`scripts/verify-macos-bundle.sh --expect-arch arm64` passes, asserting that
**both** the GUI executable and the bundled orchestration binary are arm64 — the
same architecture, so no x86_64-only daemon can be shipped inside an arm64 app.

---

## 5. Bundle verification (structure and code signature)

`scripts/verify-macos-bundle.sh` reports:

```text
ok   Contents/Info.plist exists
ok   CFBundleIdentifier = com.service-orchestrator.desktop
ok   CFBundleShortVersionString = 0.1.0
ok   main executable Contents/MacOS/service-orchestrator-desktop (arm64)
ok   bundled orchestration binary Contents/MacOS/service-orchestrator (arm64)

Code signature:
  status: AD-HOC SIGNED (sealed locally; not a distribution identity).
  ok   codesign --verify --deep --strict passed (the bundle is sealed)

ok   hdiutil recognises the disk image
ok   DMG mounted read-only
ok   contains Service Orchestrator.app
ok   mounted bundle still carries the orchestration binary
ok   contains an Applications shortcut

Verification passed.
```

### Signature detail

```text
$ codesign -dv --verbose=4 "…/Service Orchestrator.app"
Identifier=com.service-orchestrator.desktop
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20500 flags=0x10002(adhoc,runtime)
Signature=adhoc
$ codesign --verify --deep --strict "…/Service Orchestrator.app"
… valid on disk
… satisfies its Designated Requirement
$ codesign -d --entitlements - "…/Service Orchestrator.app"
  com.apple.security.cs.allow-jit
  com.apple.security.cs.allow-unsigned-executable-memory
```

The sidecar is signed too (Tauri signs every executable in the bundle), so the
bundle seal covers it. The hardened-runtime flag is present and exactly the two
WKWebView entitlements are granted.

> **Fixed during this task.** An unsigned Tauri build leaves the *bundle*
> unsealed: the executables carry a linker-generated ad-hoc signature but there
> is no `_CodeSignature/CodeResources`, so `codesign --verify --deep --strict`
> fails with *"code has no resources but signature indicates they must be
> present"* and Gatekeeper can report the app as **damaged** rather than merely
> unidentified. `scripts/build-macos.sh` now seals the bundle by ad-hoc signing
> it (identity `-`) when no Developer ID identity is configured, which needs no
> Apple credentials and makes the two cases behave as documented.

---

## 6. Code signing status

**Ad-hoc signed, not signed for distribution.** There is no Developer ID
Application certificate on the build machine, so the bundle is sealed ad-hoc
(identity `-`) by `scripts/build-macos.sh`. `codesign --verify --deep --strict`
passes because the bundle is internally consistent, but Gatekeeper still
rejects it:

```text
$ spctl -a -vvv -t exec "…/Service Orchestrator.app"
…: rejected   (exit 3)
```

To produce a distributable build, export
`APPLE_SIGNING_IDENTITY="Developer ID Application: … (TEAMID)"` and rebuild; the
identity overrides the ad-hoc default and Tauri signs with the hardened runtime
and the entitlements in `apps/desktop/src-tauri/entitlements.plist`.

---

## 7. Notarization status

**Not notarized.** Notarization needs a paid Apple Developer account and
contacts Apple's servers, so it was not attempted without credentials. Tauri
reported:

```text
Warn skipping app notarization, no APPLE_ID & APPLE_PASSWORD & APPLE_TEAM_ID
or APPLE_API_KEY & APPLE_API_ISSUER & APPLE_API_KEY_PATH environment variables found
```

Provide those variables (see [`macos-distribution.md`](macos-distribution.md)
§11) and rebuild to sign and notarize; the verification script reports the
result rather than requiring it.

---

## 8. Installation test results (DMG)

The DMG was mounted, inspected, and installed into a clean location:

| Check | Result |
| --- | --- |
| `hdiutil verify` | checksum **VALID** |
| Mounts read-only | yes |
| Contains `Service Orchestrator.app` | yes |
| Contains an `Applications` shortcut | yes — symlink to `/Applications` |
| Volume icon | `.VolumeIcon.icns` present |
| Copy to a clean `Applications/` directory | succeeded |
| `codesign --verify --deep --strict` after copy | **OK** |
| Launch the installed copy | daemon reports `running` within ~0.5 s |
| Installed binary version | `service-orchestrator 0.1.0` |
| Clean shutdown from the installed copy | `shutdown requested` → `stopped` |

The installed application runs from a directory with no connection to the build
tree, so the bundle is self-contained.

---

## 9. Runtime test results (packaged daemon)

### Bundled daemon outside the development directory

The bundled `service-orchestrator` was started from inside the `.app` against a
temporary workspace with a **clean environment** (`env -i HOME=… PATH=/usr/bin:/bin`):

- `daemon status` reports `running`, `protocol version: 1`;
- the runtime directory is created `0700`, the socket `srw-------`, the log
  `-rw-------`;
- `daemon stop` shuts it down and `daemon status` then reports `stopped`.

The IPC endpoint and log live beside the configuration file
(`<config_dir>/run/daemon.{sock,pid,log}`); the default configuration
(`~/.service-orchestrator/config.toml`) therefore uses
`~/.service-orchestrator/run/`. Nothing is written into the `.app`.

### GUI first launch: bundled binary resolution, auto-start, and IPC

The GUI executable was launched with the same clean environment and an isolated
`SERVICE_ORCHESTRATOR_CONFIG`:

- the GUI resolved the **bundled sibling** `Contents/MacOS/service-orchestrator`
  (visible in the process tree: the daemon was spawned as a child of the GUI);
- the daemon socket appeared within **~0.3 s**; the GUI connected over the Unix
  domain socket (`clients: 2` — GUI plus the status probe);
- the daemon's `PATH` was **augmented** even though the GUI inherited only
  `/usr/bin:/bin`:

  ```text
  PATH=/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin:/usr/sbin:/sbin
  ```

- after the GUI process was killed, the daemon **kept running** (`clients: 1`),
  and was then stopped explicitly. Closing the GUI does not stop the daemon or
  the services it manages.

No login shell was executed; the augmentation is static, from
`crates/client/src/path_env.rs`.

### Interfaces over the packaged daemon

The daemon is the single authoritative engine, so the packaged GUI, the TUI,
and the MCP server share one state. `service-orchestrator daemon`,
`service-orchestrator tui`, and `service-orchestrator mcp` all still work from
the bundled binary; the GUI, the terminal interfaces, and MCP attach to the same
socket.

---

## 10. Scripts created or changed

| Script | Purpose |
| --- | --- |
| `scripts/build-macos.sh` | Validate the environment, build the orchestration binary per architecture, stage the Tauri sidecar, seal/sign the bundle (ad-hoc by default), build `.app` + `.dmg`, verify. |
| `scripts/verify-macos-bundle.sh` | Verify bundle structure, metadata, both executables and their architecture, the code signature (sealed ad-hoc or Developer ID), and the mounted DMG. |
| `apps/desktop/scripts/preflight.mjs` | Now also stages the sidecar so `tauri-build` can compile the desktop crate. |

`scripts/test-e2e.sh` was updated to stage the sidecar before the desktop tests.

---

## 11. Tests added

- **`apps/desktop/src-tauri/src/daemon_bin.rs`** — binary resolution:
  `an_explicit_override_wins_when_it_exists`,
  `a_missing_override_yields_nothing_instead_of_a_bad_spawn`,
  `path_is_searched_when_no_bundled_daemon_exists`,
  `a_bundled_sibling_wins_over_a_daemon_on_path`,
  `a_daemon_in_bundle_resources_is_found`,
  `a_packaged_layout_ignores_a_target_directory_further_up`,
  `a_sibling_daemon_is_found_next_to_the_program`,
  `target_directories_are_walked_up_from_a_nested_binary`,
  `nothing_found_is_reported_honestly`,
  `the_env_override_name_is_the_client_contract`.
- **`crates/client/src/path_env.rs`** — `PATH` augmentation:
  `inherited_entries_come_first_and_are_never_dropped`,
  `missing_directories_are_skipped`, `duplicates_are_not_added_twice`,
  `an_empty_inherited_path_still_gets_the_discovered_entries`,
  `discovery_includes_java_home_and_home_relative_managers`,
  `nvm_node_bins_are_discovered_newest_first`,
  `effective_search_path_keeps_the_live_path`.
- **`crates/client/src/socket.rs`** —
  `the_spawned_daemon_receives_the_socket_and_an_augmented_path`.

Existing daemon/IPC integration tests in
`apps/desktop/src-tauri/tests/daemon_ipc.rs` continue to cover the packaged
GUI → daemon → application-core flow against isolated fixtures.

---

## 12. Reproducing

```bash
./scripts/build-macos.sh --arch arm64    # build + verify
./scripts/verify-macos-bundle.sh --expect-arch arm64
```

---

## 13. Known limitations

- **Verified architecture is Apple Silicon only.** `scripts/build-macos.sh`
  supports `--arch x86_64` and `--arch universal` when the matching Rust
  targets are installed, but this machine had no `rustup` and no
  `x86_64-apple-darwin` standard library, so neither was produced or verified.
  Do not claim Intel or universal support until a build is actually made and
  checked with `--expect-arch x86_64` / `--expect-arch universal`.
- **The reference build is ad-hoc signed and not notarized**, so Gatekeeper warns
  on first launch. This is expected and documented; nothing disables Gatekeeper.
- **The application icon is an original placeholder**, not final branding
  (`apps/desktop/icon-source.png`).
- **No automatic updates and no CI/CD** — distribution is a local DMG, and a new
  version is installed manually while user configuration and workspace data are
  preserved.
- **`rust-objcopy` debug-info stripping emits a non-fatal warning** because the
  toolchain in this environment is missing `libLLVM.dylib`; the bundled binaries
  are larger than a stripped release would be but are otherwise correct.

---

## 14. See also

- [`macos-distribution.md`](macos-distribution.md) — build, verify, install, and distribute.
- [`desktop-gui.md`](desktop-gui.md) — GUI internals, including binary resolution.
- [`known-limitations.md`](known-limitations.md) — the packaging limitations above, in context.
- [`local-daemon.md`](local-daemon.md) — the shared runtime and IPC.
