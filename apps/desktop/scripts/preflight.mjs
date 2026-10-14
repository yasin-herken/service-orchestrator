#!/usr/bin/env node
/**
 * Preflight for the desktop app's Tauri commands (`npm run tauri dev` /
 * `npm run tauri:build`) and for the desktop test script.
 *
 * Two things must exist before the desktop crate compiles:
 *
 *  1. The **daemon host binary**. The GUI's daemon auto-start re-executes the
 *     `service-orchestrator` binary (`SERVICE_ORCHESTRATOR_DAEMON_BIN`, see
 *     crates/client/src/socket.rs), which lives in the Cargo workspace at the
 *     repository root and may not have been built yet on a fresh checkout.
 *  2. The **Tauri external binary (sidecar)**. `tauri.conf.json` declares
 *     `bundle.externalBin = ["binaries/service-orchestrator"]`, so `tauri-build`
 *     refuses to compile until a file named
 *     `binaries/service-orchestrator-<target-triple>` exists.
 *
 * Both are satisfied by building the orchestration binary once and copying it to
 * the triple-named sidecar path. This keeps the Task 015 auto-start mechanism
 * authoritative: the GUI never spawns daemons itself.
 *
 * Usage:
 *   node scripts/preflight.mjs [--profile debug|release] [--stage-only]
 *
 * `--profile` selects the Cargo profile (default `debug`, which is what `tauri
 * dev` and the desktop tests use). Release packaging goes through
 * `scripts/build-macos.sh`, which stages a per-architecture release sidecar
 * before invoking `tauri build` directly. `--stage-only` stops after staging
 * (used by scripts/test-e2e.sh).
 */
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, chmodSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const desktopRoot = resolve(here, '..');
const srcTauri = join(desktopRoot, 'src-tauri');
const repoRoot = resolve(here, '..', '..', '..');
const manifest = join(repoRoot, 'Cargo.toml');
const binariesDir = join(srcTauri, 'binaries');

function parseArgs(argv) {
  let profile = 'debug';
  let stageOnly = false;
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--profile') {
      profile = argv[i + 1] ?? '';
      i += 1;
    } else if (arg.startsWith('--profile=')) {
      profile = arg.slice('--profile='.length);
    } else if (arg === '--stage-only') {
      stageOnly = true;
    } else {
      console.error(`[desktop] unknown argument: ${arg}`);
      process.exit(2);
    }
  }
  if (profile !== 'debug' && profile !== 'release') {
    console.error(`[desktop] --profile must be 'debug' or 'release' (got '${profile}')`);
    process.exit(2);
  }
  return { profile, stageOnly };
}

function hostTarget() {
  const result = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
  if (result.error) {
    console.error(`[desktop] could not run rustc: ${result.error.message}`);
    console.error('[desktop] install a Rust toolchain (https://rustup.rs) and try again.');
    process.exit(1);
  }
  const match = /^host: (.+)$/m.exec(result.stdout ?? '');
  if (!match) {
    console.error('[desktop] could not determine the Rust host target from `rustc -vV`.');
    process.exit(1);
  }
  return match[1].trim();
}

function main() {
  const { profile, stageOnly } = parseArgs(process.argv.slice(2));
  const target = hostTarget();

  const cargoArgs = ['build', '--manifest-path', manifest, '--bin', 'service-orchestrator'];
  if (profile === 'release') {
    cargoArgs.push('--release');
  }
  console.log(`[desktop] building the service-orchestrator binary (${profile})…`);
  const result = spawnSync('cargo', cargoArgs, { stdio: 'inherit', cwd: repoRoot });
  if (result.error) {
    console.error(`[desktop] could not run cargo: ${result.error.message}`);
    console.error('[desktop] install a Rust toolchain (https://rustup.rs) and try again.');
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[desktop] \`cargo ${cargoArgs.join(' ')}\` failed; fix the build and retry.`);
    process.exit(result.status ?? 1);
  }

  const built = join(repoRoot, 'target', profile, 'service-orchestrator');
  const sidecar = join(binariesDir, `service-orchestrator-${target}`);
  mkdirSync(binariesDir, { recursive: true });
  copyFileSync(built, sidecar);
  chmodSync(sidecar, 0o755);
  console.log(`[desktop] staged sidecar ${sidecar}`);

  if (stageOnly) {
    return;
  }
  console.log('[desktop] preflight complete.');
}

main();
