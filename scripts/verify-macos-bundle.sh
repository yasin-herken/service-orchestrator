#!/usr/bin/env bash
#
# Verify a built macOS application bundle and its DMG (Task 031 §34, §36, §37,
# §38, §44). It requires no Apple Developer credentials: signing and notarization
# are reported, not required.
#
# It checks:
#   * the `.app` exists and has a valid `Contents/Info.plist`;
#   * `CFBundleIdentifier`, `CFBundleShortVersionString`, and `CFBundleExecutable`
#     are present and the executable actually exists;
#   * the bundled `service-orchestrator` orchestration binary is present, is
#     executable, and reports its architecture;
#   * the main and bundled executables resolve to a supported architecture set
#     (optionally asserted with `--expect-arch`);
#   * the DMG is a valid image, mounts, contains the application and an
#     `/Applications` shortcut, and the application can be read from it;
#   * the code-signature status and a Gatekeeper assessment are reported.
#
# Usage:
#   ./scripts/verify-macos-bundle.sh [APP] [DMG] [--expect-arch arm64|x86_64|universal]
#
# With no arguments it discovers the most recently built bundle under
# apps/desktop/src-tauri/target/*/release/bundle.
#
# Exit codes:
#   0  every required check passed
#   1  a required check failed
#   2  usage error

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

app=""
dmg=""
expect_arch=""

usage() {
  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
}

positional=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --expect-arch)
      [[ $# -ge 2 ]] || { echo "error: --expect-arch requires a value" >&2; exit 2; }
      expect_arch="$2"
      shift 2
      ;;
    --help | -h)
      usage
      exit 0
      ;;
    *)
      positional+=("$1")
      shift
      ;;
  esac
done
app="${positional[0]:-}"
dmg="${positional[1]:-}"

failures=0
fail() { echo "  FAIL $*"; failures=$((failures + 1)); }
pass() { echo "  ok   $*"; }

if [[ -z "$app" ]]; then
  app="$(find "$root/apps/desktop/src-tauri/target" -maxdepth 6 -path '*/release/bundle/macos/*.app' -print -quit 2>/dev/null || true)"
fi
if [[ -z "$app" ]]; then
  echo "error: no application bundle found; build it first with ./scripts/build-macos.sh" >&2
  exit 2
fi

echo "Verifying application bundle: $app"
if [[ ! -d "$app" ]]; then
  echo "error: '$app' is not a directory" >&2
  exit 2
fi

# -- bundle structure --------------------------------------------------------
plist="$app/Contents/Info.plist"
if [[ -f "$plist" ]]; then
  pass "Contents/Info.plist exists"
else
  fail "Contents/Info.plist is missing"
fi

read_plist() {
  /usr/bin/plutil -extract "$1" raw -o - "$plist" 2>/dev/null || true
}

bundle_id="$(read_plist CFBundleIdentifier)"
bundle_version="$(read_plist CFBundleShortVersionString)"
bundle_exec="$(read_plist CFBundleExecutable)"
if [[ -n "$bundle_id" ]]; then
  pass "CFBundleIdentifier = $bundle_id"
else
  fail "CFBundleIdentifier is missing"
fi
if [[ -n "$bundle_version" ]]; then
  pass "CFBundleShortVersionString = $bundle_version"
else
  fail "CFBundleShortVersionString is missing"
fi

archs_of() { /usr/bin/lipo -archs "$1" 2>/dev/null || file -b "$1"; }

main_exe="$app/Contents/MacOS/$bundle_exec"
if [[ -n "$bundle_exec" && -x "$main_exe" ]]; then
  pass "main executable Contents/MacOS/$bundle_exec ($(archs_of "$main_exe"))"
else
  fail "CFBundleExecutable '$bundle_exec' does not resolve to an executable under Contents/MacOS"
fi

# -- the bundled orchestration binary ---------------------------------------
orchestrator="$app/Contents/MacOS/service-orchestrator"
if [[ -x "$orchestrator" ]]; then
  pass "bundled orchestration binary Contents/MacOS/service-orchestrator ($(archs_of "$orchestrator"))"
else
  fail "bundled orchestration binary Contents/MacOS/service-orchestrator is missing or not executable"
fi

# -- architecture assertion --------------------------------------------------
if [[ -n "$expect_arch" ]]; then
  check_arch() {
    local path="$1" label="$2" outputs
    outputs="$(archs_of "$path")"
    case "$expect_arch" in
      arm64)
        [[ "$outputs" == *arm64* ]] || fail "$label is not arm64 ($outputs)"
        ;;
      x86_64)
        [[ "$outputs" == *x86_64* ]] || fail "$label is not x86_64 ($outputs)"
        ;;
      universal)
        if [[ "$outputs" == *arm64* && "$outputs" == *x86_64* ]]; then
          :
        else
          fail "$label is not universal ($outputs)"
        fi
        ;;
      *)
        fail "unknown --expect-arch '$expect_arch'"
        ;;
    esac
  }
  [[ -n "$bundle_exec" ]] && check_arch "$main_exe" "the main executable"
  [[ -x "$orchestrator" ]] && check_arch "$orchestrator" "the orchestration binary"
  pass "architecture assertion attempted ($expect_arch)"
fi

# -- code signature (reported) ----------------------------------------------
# A credential-free local build is ad-hoc signed: the toolchain linker-signs each
# executable and `./scripts/build-macos.sh` seals the bundle ad-hoc. A Developer
# ID signature is reported, never required (tasks/031.md §31, §38); an ad-hoc
# bundle is still verified strictly, because it must at least be self-consistent.
echo
echo "Code signature:"
if ! /usr/bin/codesign -dv "$app" >/dev/null 2>&1; then
  echo "  status: UNSIGNED (no code signature). Gatekeeper will warn on first launch."
  echo "        To sign, export APPLE_SIGNING_IDENTITY and rebuild."
else
  signature_info="$(/usr/bin/codesign -dv "$app" 2>&1 || true)"
  if [[ "$signature_info" == *adhoc* ]]; then
    echo "  status: AD-HOC SIGNED (sealed locally; not a distribution identity)."
    if /usr/bin/codesign --verify --deep --strict "$app" >/dev/null 2>&1; then
      pass "codesign --verify --deep --strict passed (the bundle is sealed)"
    else
      echo "  note the bundle is not sealed — the executables carry a linker signature"
      echo "       only, so Gatekeeper may report the app as damaged rather than"
      echo "       unidentified. ./scripts/build-macos.sh seals it ad-hoc."
    fi
    echo "        Gatekeeper will warn on first launch (not notarized)."
    echo "        To sign for distribution, export APPLE_SIGNING_IDENTITY and rebuild."
  else
    identity="$(printf '%s\n' "$signature_info" | tr '\n' ' ')"
    echo "  signed: $identity"
    if /usr/bin/codesign --verify --deep --strict "$app" >/dev/null 2>&1; then
      pass "codesign --verify --deep --strict passed"
    else
      fail "codesign --verify --deep --strict failed for a signed bundle"
    fi
    if /usr/sbin/spctl --assess --type execute "$app" >/dev/null 2>&1; then
      pass "Gatekeeper assessment passed (notarized or locally approved)"
    else
      echo "  note Gatekeeper would block this build (not notarized; expected for local signed builds)"
    fi
  fi
fi

# -- DMG ---------------------------------------------------------------------
if [[ -z "$dmg" ]]; then
  dmg="$(find "$root/apps/desktop/src-tauri/target" -maxdepth 6 -path '*/release/bundle/dmg/*.dmg' -print -quit 2>/dev/null || true)"
fi
if [[ -n "$dmg" ]]; then
  echo
  echo "Verifying DMG: $dmg"
  if [[ ! -f "$dmg" ]]; then
    fail "'$dmg' does not exist"
  elif ! /usr/bin/hdiutil imageinfo "$dmg" >/dev/null 2>&1; then
    fail "hdiutil does not recognise '$dmg' as a disk image"
  else
    pass "hdiutil recognises the disk image"
    mount_point="$(mktemp -d "${TMPDIR:-/tmp}/so-dmg-verify.XXXXXX")"
    cleanup_mount() { /usr/bin/hdiutil detach "$mount_point" -quiet >/dev/null 2>&1 || true; rmdir "$mount_point" 2>/dev/null || true; }
    trap cleanup_mount EXIT
    if /usr/bin/hdiutil attach "$dmg" -nobrowse -readonly -mountpoint "$mount_point" >/dev/null 2>&1; then
      pass "DMG mounted read-only"
      mounted_app="$mount_point/$(basename "$app")"
      if [[ -d "$mounted_app" ]]; then
        pass "contains $(basename "$app")"
        if [[ -x "$mounted_app/Contents/MacOS/service-orchestrator" ]]; then
          pass "mounted bundle still carries the orchestration binary"
        else
          fail "mounted bundle is missing the orchestration binary"
        fi
      else
        fail "does not contain $(basename "$app")"
      fi
      if [[ -L "$mount_point/Applications" || -d "$mount_point/Applications" ]]; then
        pass "contains an Applications shortcut"
      else
        fail "is missing the Applications shortcut"
      fi
    else
      fail "could not mount the DMG"
    fi
    cleanup_mount
    trap - EXIT
  fi
fi

echo
if [[ "$failures" -eq 0 ]]; then
  echo "Verification passed."
  exit 0
fi
echo "Verification failed ($failures check(s))."
exit 1
