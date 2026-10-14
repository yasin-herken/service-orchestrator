#!/usr/bin/env bash
#
# Build a distributable macOS application and DMG for Service Orchestrator
# (Task 031). This is a **local** release script: it uses the repository's own
# toolchain, writes into the repository's own `target/` directories, and never
# talks to CI, a registry, or a remote deployment target.
#
# It performs, in order:
#   1. validates the build environment (cargo, node, npm, Xcode command line
#      tools, and the requested Rust target(s));
#   2. verifies the application version is consistent across Cargo and Tauri;
#   3. builds the `service-orchestrator` orchestration binary for the requested
#      architecture(s);
#   4. stages it as the Tauri external binary (sidecar) with the target-triple
#      name Tauri expects;
#   5. builds the Tauri desktop app and bundles `Service Orchestrator.app` and
#      `Service Orchestrator_<version>_<arch>.dmg`, ad-hoc signing the bundle
#      when no Developer ID identity is set;
#   6. verifies the artifacts exist, checks each bundled executable's
#      architecture with `file`/`lipo`, and reports the output paths.
#
# Signing/notarization are driven entirely by the standard environment Tauri
# understands (`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID`,
# `APPLE_PASSWORD`, or the App Store Connect API key variables). With neither a
# Developer ID identity nor notarization credentials set, the script ad-hoc signs
# the bundle (so it is internally consistent and Gatekeeper shows the normal
# "unidentified developer" prompt instead of reporting a damaged app) and
# produces an unnotarized build; it says so. It never stores a credential.
#
# Usage:
#   ./scripts/build-macos.sh                  # native Apple Silicon (arm64)
#   ./scripts/build-macos.sh --arch arm64
#   ./scripts/build-macos.sh --arch x86_64
#   ./scripts/build-macos.sh --arch universal
#   ./scripts/build-macos.sh --debug          # debug profile (faster, larger)
#
# Exit codes:
#   0  success
#   1  a build step failed
#   2  usage error or a missing build prerequisite

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

arch="arm64"
profile="release"
profile_flag="--release"
tauri_profile_flag=""

usage() {
  sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)
      [[ $# -ge 2 ]] || { echo "error: --arch requires a value" >&2; exit 2; }
      arch="$2"
      shift 2
      ;;
    --arch=*)
      arch="${1#--arch=}"
      shift
      ;;
    --debug)
      profile="debug"
      profile_flag=""
      tauri_profile_flag="--debug"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument '$1'" >&2
      echo "run './scripts/build-macos.sh --help' for usage" >&2
      exit 2
      ;;
  esac
done

case "$arch" in
  arm64 | x86_64 | universal) ;;
  *)
    echo "error: --arch must be one of: arm64, x86_64, universal (got '$arch')" >&2
    exit 2
    ;;
esac

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: '$1' is required but was not found on PATH." >&2
    echo "       $2" >&2
    exit 2
  fi
}

echo "==> Service Orchestrator macOS build (arch: $arch, profile: $profile)"

# -- 1. environment ----------------------------------------------------------
require cargo "Install a Rust toolchain (https://rustup.rs)."
require node "Install Node.js 20.19+ (see README.md)."
require npm "npm ships with Node.js."
require file "This is a macOS system tool; is Xcode Command Line Tools installed?"
if ! xcode-select -p >/dev/null 2>&1; then
  echo "error: Xcode Command Line Tools are not installed." >&2
  echo "       Run: xcode-select --install" >&2
  exit 2
fi

host_target="$(rustc -vV | sed -n 's/^host: //p')"
echo "    host rust target: $host_target"

case "$arch" in
  arm64) targets=("aarch64-apple-darwin") ;;
  x86_64) targets=("x86_64-apple-darwin") ;;
  universal) targets=("aarch64-apple-darwin" "x86_64-apple-darwin") ;;
esac

# Tauri identifies a build by the Rust target triple, not by the friendly
# `--arch` name; `arm64` is the Rust `aarch64-apple-darwin`.
case "$arch" in
  arm64) tauri_target="aarch64-apple-darwin" ;;
  x86_64) tauri_target="x86_64-apple-darwin" ;;
  universal) tauri_target="universal-apple-darwin" ;;
esac

# Every requested target must have its standard library installed. Without
# rustup we cannot add it, so the message names the exact command.
for target in "${targets[@]}"; do
  libdir="$(rustc --print target-libdir --target "$target" 2>/dev/null || true)"
  if [[ -z "$libdir" || ! -d "$libdir" ]]; then
    echo "error: the Rust target '$target' is not installed." >&2
    echo "       Install it with: rustup target add $target" >&2
    exit 2
  fi
done

# -- 2. version consistency --------------------------------------------------
cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
tauri_version="$(node -e "process.stdout.write(require('./apps/desktop/src-tauri/tauri.conf.json').version)")"
if [[ "$cargo_version" != "$tauri_version" ]]; then
  echo "error: version mismatch: Cargo.toml=$cargo_version tauri.conf.json=$tauri_version" >&2
  echo "       One authoritative version is required (tasks/031.md §28)." >&2
  exit 1
fi
echo "    application version: $cargo_version"

# -- 3. orchestration binary -------------------------------------------------
src_tauri="$root/apps/desktop/src-tauri"
binaries_dir="$src_tauri/binaries"
mkdir -p "$binaries_dir"

for target in "${targets[@]}"; do
  echo "==> Building the orchestration binary for $target"
  # shellcheck disable=SC2086
  cargo build $profile_flag --target "$target" --bin service-orchestrator
  built="$root/target/$target/$profile/service-orchestrator"
  if [[ ! -f "$built" ]]; then
    echo "error: expected binary not found at $built" >&2
    exit 1
  fi
  sidecar="$binaries_dir/service-orchestrator-$target"
  cp "$built" "$sidecar"
  chmod +x "$sidecar"
  echo "    staged sidecar: ${sidecar#$root/} ($(file -b "$sidecar" | cut -d, -f1))"
done

# -- 4. frontend dependencies ------------------------------------------------
if [[ ! -d "$root/apps/desktop/node_modules" ]]; then
  echo "==> Installing desktop dependencies (npm ci)"
  (cd "$root/apps/desktop" && npm ci)
fi

# -- 5. signing default ------------------------------------------------------
# macOS linker-signs every executable ad-hoc, but that leaves the *bundle*
# unsealed: `codesign --verify --deep --strict` fails with "code has no resources
# but signature indicates they must be present", and Gatekeeper reports the app
# as damaged rather than merely unidentified. Ad-hoc signing the whole bundle
# (identity `-`) makes it self-consistent with no Apple credentials. A real
# Developer ID identity always takes precedence; neither is notarized.
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  export APPLE_SIGNING_IDENTITY="-"
  signing_status="ad-hoc (sealed, but not a distribution identity; unnotarized)"
else
  signing_status="${APPLE_SIGNING_IDENTITY}"
fi
echo "    signing: ${signing_status}"

# -- 6. Tauri bundle ---------------------------------------------------------
echo "==> Building the Tauri application and bundles ($tauri_target, profile: $profile)"
# shellcheck disable=SC2086
(cd "$root/apps/desktop" && npx tauri build $tauri_profile_flag --target "$tauri_target" --bundles app,dmg)

bundle_dir="$root/apps/desktop/src-tauri/target/$tauri_target/$profile/bundle"
app_path="$bundle_dir/macos/Service Orchestrator.app"
dmg_dir="$bundle_dir/dmg"

# -- 7. verification ---------------------------------------------------------
if [[ ! -d "$app_path" ]]; then
  echo "error: the application bundle was not produced at $app_path" >&2
  exit 1
fi
dmg_path="$(find "$dmg_dir" -maxdepth 1 -name '*.dmg' -print -quit 2>/dev/null || true)"
if [[ -z "$dmg_path" || ! -f "$dmg_path" ]]; then
  echo "error: no DMG was produced under $dmg_dir" >&2
  exit 1
fi

echo "==> Verifying the bundle"
"$root/scripts/verify-macos-bundle.sh" "$app_path" "$dmg_path"

echo
echo "==> Build complete"
echo "    app:  $app_path"
echo "    dmg:  $dmg_path"
echo "    signing: ${signing_status}"
if [[ "${APPLE_SIGNING_IDENTITY}" == "-" ]]; then
  echo "             Gatekeeper still warns (unnotarized). Set APPLE_SIGNING_IDENTITY"
  echo "             to a Developer ID to produce a distributable, notarizable build."
fi
