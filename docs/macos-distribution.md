# macOS Distribution

This document describes how to build, verify, install, and hand out the
**Service Orchestrator** macOS application (`Service Orchestrator.app` and
`Service Orchestrator_<version>_<arch>.dmg`). It is the operational guide for
Task 031; the measured results of the reference build are in
[`packaging-report.md`](packaging-report.md).

The distribution is entirely **local**: a developer builds the DMG on their
machine and hands the file to a teammate. There is no CI, no registry, no cloud
upload, and no automatic update mechanism.

---

## 1. What is in the bundle

```text
Service Orchestrator.app
└── Contents
    ├── Info.plist                     # identity, version, minimum macOS
    ├── MacOS
    │   ├── service-orchestrator-desktop   # the Tauri GUI executable
    │   └── service-orchestrator           # the bundled orchestration binary
    ├── Resources
    │   └── icon.icns                      # application icon
    └── _CodeSignature
        └── CodeResources                  # bundle seal (ad-hoc by default)
```

The GUI is a **thin adapter**: it holds no orchestration engine of its own. The
bundled `service-orchestrator` binary is the single authoritative implementation
of every operation, and it still supports its three original modes:

```bash
service-orchestrator daemon    # the shared local runtime (started by the GUI)
service-orchestrator tui       # the terminal interface
service-orchestrator mcp       # the MCP stdio server
```

The GUI auto-starts the bundled binary as the shared daemon; a terminal can
attach to the very same daemon (`docs/local-daemon.md`), so a task started in
the GUI is visible in the TUI and MCP and vice versa.

---

## 2. Supported macOS versions and architectures

| Item | Value |
| --- | --- |
| Minimum macOS | **11.0 (Big Sur)** — `bundle.macOS.minimumSystemVersion` |
| Primary architecture | **Apple Silicon** (`aarch64-apple-darwin`) |
| Intel architecture | `x86_64-apple-darwin` — supported by the build script when the Rust target is installed |
| Universal binary | `universal-apple-darwin` — supported by the build script when **both** Rust targets are installed |

The application is a native binary; it is not a script, an Electron app, or a
bundled Python runtime. It never ships a Rust or Node toolchain.

> **Verified in the reference build:** Apple Silicon only. The build machine had
> only the `aarch64-apple-darwin` Rust standard library installed and no
> `rustup`, so an Intel or universal build could not be produced or verified
> there. See [`packaging-report.md`](packaging-report.md) §"Known limitations".

---

## 3. Build prerequisites

Building the distribution (not running it) needs the repository's developer
toolchain:

| Tool | Why | Check |
| --- | --- | --- |
| Rust (MSRV 1.88) | builds the orchestration binary and the GUI | `cargo --version` |
| Node.js 20.19+ and npm | builds the React frontend | `node --version`, `npm --version` |
| Xcode Command Line Tools | `codesign`, `lipo`, `hdiutil`, the macOS linker | `xcode-select -p` |
| The requested Rust **target(s)** | cross/universal builds | `rustc --print target-libdir --target x86_64-apple-darwin` |

For an Intel or universal build, install the extra target first:

```bash
rustup target add x86_64-apple-darwin      # Intel
rustup target add aarch64-apple-darwin x86_64-apple-darwin   # universal
```

Installing the **application** needs none of these — only macOS.

---

## 4. Building locally

The script wraps the whole flow:

```bash
./scripts/build-macos.sh                   # native Apple Silicon (arm64)
./scripts/build-macos.sh --arch arm64
./scripts/build-macos.sh --arch x86_64
./scripts/build-macos.sh --arch universal
./scripts/build-macos.sh --debug           # faster, larger, debug profile
```

It validates the environment, checks that the application version agrees between
`Cargo.toml` and `tauri.conf.json`, builds the orchestration binary for each
requested target, stages it as the Tauri external binary, builds the Tauri app,
bundles the `.app` and `.dmg`, and finally runs the verification script.

The equivalent manual steps, if you prefer to drive them yourself:

```bash
# 1. Orchestration binary (per architecture).
cargo build --release --target aarch64-apple-darwin --bin service-orchestrator

# 2. Stage it where Tauri's `bundle.externalBin` expects it.
mkdir -p apps/desktop/src-tauri/binaries
cp target/aarch64-apple-darwin/release/service-orchestrator \
   apps/desktop/src-tauri/binaries/service-orchestrator-aarch64-apple-darwin

# 3. Frontend dependencies (once).
cd apps/desktop && npm ci && cd ../..

# 4. Tauri build (runs `npm run build`, then bundles app + dmg).
cd apps/desktop
npx tauri build --target aarch64-apple-darwin --bundles app,dmg
```

Tauri looks for the external binary named `service-orchestrator-<target-triple>`
and places the stripped binary at `Contents/MacOS/service-orchestrator` inside
the bundle.

---

## 5. Artifact locations

```text
apps/desktop/src-tauri/target/<triple>/release/bundle/
├── macos/Service Orchestrator.app
└── dmg/Service Orchestrator_<version>_<arch>.dmg
```

`<triple>` is `aarch64-apple-darwin`, `x86_64-apple-darwin`, or
`universal-apple-darwin`. The `<arch>` suffix in the DMG name is `aarch64`,
`x64`, or `universal`, as Tauri writes it.

The staged external binaries under `apps/desktop/src-tauri/binaries/` are build
inputs and are not committed (see `.gitignore`).

---

## 6. Installing from the DMG

1. Double-click `Service Orchestrator_<version>_<arch>.dmg`.
2. Drag **Service Orchestrator** onto the **Applications** shortcut.
3. Eject the disk image.
4. Launch **Service Orchestrator** from `/Applications` (or Launchpad).

No administrator password is required and nothing is installed outside
`/Applications`. User data lives in `~/.service-orchestrator/` (see §8 below),
so replacing the application later preserves configuration and workspace data.

---

## 7. First launch and daemon behaviour

```text
Launch from Finder
      │
      ▼
GUI starts, resolves the bundled service-orchestrator binary
      │
      ▼
Is a compatible daemon already running for this configuration?
      │ no                         │ yes
      ▼                            ▼
auto-start `service-orchestrator   attach to it
daemon` (detached child)                    │
      └──────────────┬──────────────────────┘
                     ▼
        wait for the socket, connect, load the active workspace,
        render the dashboard
```

- The GUI **never** terminates or replaces a daemon it did not start. A daemon
  already running from a terminal is reused; a version mismatch surfaces as an
  actionable `PROTOCOL_INCOMPATIBLE` error instead of a silent restart.
- The GUI **never** re-executes its own executable as a daemon. It resolves the
  real `service-orchestrator` binary inside the bundle first, then `PATH`, then
  the development `target/` layout (see `docs/desktop-gui.md` §6).
- **Closing the window does not stop the daemon or the services it manages.**
  The daemon is a detached child in its own process group. Stop it explicitly
  with `service-orchestrator daemon stop`, or `service-orchestrator daemon
  status` to check on it.

If the GUI cannot find the bundled binary, cannot start the daemon, or cannot
connect, it opens with a connection banner and a Retry action, and shows the
daemon log path (`~/.service-orchestrator/run/daemon.log`) as a hint.

---

## 8. Filesystem locations

The application is **local-first** and writes only to the user's home directory:

| Path | Purpose |
| --- | --- |
| `~/.service-orchestrator/config.toml` | the configuration file (never overwritten by an install) |
| `~/.service-orchestrator/run/daemon.sock` | the IPC socket (`0700` directory, owner-only socket) |
| `~/.service-orchestrator/run/daemon.pid` | the daemon PID file |
| `~/.service-orchestrator/run/daemon.log` | the daemon's own log output (`0600`) |

Nothing is ever written into the signed `.app` bundle, and the application does
not use a cache or a container directory outside home. The configuration path
can be overridden with `SERVICE_ORCHESTRATOR_CONFIG`, and the socket with
`SERVICE_ORCHESTRATOR_SOCKET`.

---

## 9. Environment resolution (launched from Finder)

An application launched from Finder does not inherit a login shell's `PATH`, so
Homebrew, SDKMAN, nvm, fnm, Volta, and `JAVA_HOME/bin` are not on it. When the
GUI auto-starts the daemon it **augments the daemon's `PATH`** with the
developer tool directories that actually exist on the machine
(`crates/client/src/path_env.rs`):

- the inherited `PATH` entries first (so an explicitly configured toolchain
  always wins);
- `$JAVA_HOME/bin`;
- `~/.volta/bin`, `~/.asdf/shims`, `~/.local/share/fnm/aliases/default/bin`,
  `~/.sdkman/candidates/java/current/bin`, `~/.jenv/shims`;
- every `~/.nvm/versions/node/*/bin`, newest first;
- `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`, `/usr/sbin`,
  `/sbin`.

This is **static discovery**, not shell invocation: the application never runs
`zsh -lc`, never sources `.zshrc`, and never guesses a directory that is not
present. `validate_workspace` (the doctor) reports any managed-service tool that
is still missing, with the exact variable or command to fix.

### Application runtime vs managed-project tools

- **The application itself needs no external tool to launch.** Rust, Cargo,
  Node, and npm are build-time only.
- **Managed projects** may need `git`, `java`, `mvn`/`mvnw`, `node`, `npm`, and
  Liquibase tooling, depending entirely on the workspace configuration. The
  application never installs any of them silently.

---

## 10. Code signing

Signing with a real Apple identity is **optional**. With no Developer ID
identity configured, `scripts/build-macos.sh` still **ad-hoc signs** the bundle
(identity `-`) so that it is self-consistent: without this seal
`codesign --verify --deep --strict` fails and Gatekeeper may report the app as
*damaged*. Ad-hoc signing needs no Apple credentials and is not a distribution
signature.

To sign for distribution, obtain a **Developer ID Application** certificate in
your login keychain and export the standard variable Tauri understands:

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
./scripts/build-macos.sh
```

Tauri then signs every executable in the bundle — the GUI and the bundled
`service-orchestrator` — with the hardened runtime and the minimal entitlements
in `apps/desktop/src-tauri/entitlements.plist`. The certificate and its password
are never stored in the repository.

The entitlements file grants exactly two keys, both required by the embedded
WKWebView's JavaScript engine under the hardened runtime:

- `com.apple.security.cs.allow-jit`
- `com.apple.security.cs.allow-unsigned-executable-memory`

Nothing else is granted. See the comments in the file for the justification.

## 11. Notarization

Notarization is **optional**, requires a paid Apple Developer account, and
contacts Apple's servers, so it is never attempted automatically. Provide
credentials through the environment Tauri expects, then rebuild:

```bash
# Option A — Apple ID
export APPLE_ID="you@example.com"
export APPLE_TEAM_ID="TEAMID"
export APPLE_PASSWORD="app-specific-password"

# Option B — App Store Connect API key
export APPLE_API_ISSUER="…"
export APPLE_API_KEY="…"
export APPLE_API_KEY_PATH="/path/to/AuthKey.p8"

export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
./scripts/build-macos.sh
```

Tauri submits the signed bundle, waits for the result, and staples the ticket.
**This repository's reference build was ad-hoc signed and not notarized** — a
distribution signature needs a Developer ID certificate, and notarization needs
Apple credentials, neither of which is present in the build environment. Signing
and notarization status are always reported by
`scripts/verify-macos-bundle.sh`.

## 12. Unsigned build limitations

An ad-hoc signed, unnotarized application is still runnable, but macOS
Gatekeeper warns:

- On first launch you may see *"Service Orchestrator cannot be opened because
  the developer cannot be verified."*
- Open it with **System Settings → Privacy & Security → Open Anyway**, or
  right-click the app and choose **Open** once.
- This is expected for a local build without a Developer ID.

To keep the warning at the normal *unidentified developer* prompt, the build
ad-hoc signs and seals the bundle. An *unsealed* bundle (executables linker-signed
but no `_CodeSignature/CodeResources`) can instead be reported as **damaged**;
`scripts/verify-macos-bundle.sh` checks for the seal and says so.

The application does **not** disable Gatekeeper, does not ask you to run
`spctl --master-disable`, and does not modify system security settings. Prefer
signing and notarizing for anything you hand to someone outside your own
machine.

---

## 13. Distributing the DMG to teammates

1. Build on a Mac: `./scripts/build-macos.sh --arch arm64` (or `--arch
   universal` for a mixed fleet).
2. Verify: `./scripts/verify-macos-bundle.sh` (it reports architecture,
   signature, and DMG contents).
3. Send the `.dmg` through whatever file-sharing you already use (email, a
   shared drive, a chat tool). There is no registry or upload step.
4. The recipient installs it as in §6.

A teammate's existing `~/.service-orchestrator/config.toml` and workspace data
are untouched by re-installation.

---

## 14. Verification

```bash
# Auto-discovers the newest built bundle + dmg under target/.
./scripts/verify-macos-bundle.sh

# Explicit paths and a strict architecture assertion.
./scripts/verify-macos-bundle.sh \
  "apps/desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Service Orchestrator.app" \
  "apps/desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/Service Orchestrator_0.1.0_aarch64.dmg" \
  --expect-arch arm64
```

It checks the bundle structure and `Info.plist`, the presence and architecture
of both executables, the code-signature status, and that the DMG mounts, holds
the application and an `/Applications` shortcut, and still carries the
orchestration binary. It needs no Apple credentials.

---

## 15. Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| *"Service Orchestrator cannot be opened because the developer cannot be verified"* | Unsigned build. Right-click → Open, or System Settings → Privacy & Security → Open Anyway. |
| Dialog: *the daemon is not running and `service-orchestrator` could not be found* | The bundle lost its external binary. Rebuild; `verify-macos-bundle.sh` checks for it. |
| `PROTOCOL_INCOMPATIBLE` banner | A daemon from a different build is running for the same configuration. Stop it (`service-orchestrator daemon stop`) or point the GUI at another configuration. |
| Managed service fails to find `mvn`/`node`/`java` | The tool is not in a discovered directory. Add it to the configuration with an absolute path, use a project `mvnw`/`npm` wrapper, or install it in a standard location; then `validate_workspace`. |
| `validate_workspace` reports a tool missing that Terminal can run | The directory is not one of the discovered locations. Export `JAVA_HOME`, or move the tool to `~/.volta/bin`/`/opt/homebrew/bin`, and restart the daemon. |
| Build fails: *the Rust target 'x86_64-apple-darwin' is not installed* | `rustup target add x86_64-apple-darwin` (and `aarch64-apple-darwin` for universal). |
| DMG will not open on an older Mac | The DMG targets macOS 11.0+; older systems are unsupported. |

## 16. See also

- [`packaging-report.md`](packaging-report.md) — the reference build's measured results.
- [`desktop-gui.md`](desktop-gui.md) — the GUI internals, including binary resolution.
- [`local-daemon.md`](local-daemon.md) — the shared runtime and IPC.
- [`troubleshooting.md`](troubleshooting.md) — general diagnosis.
