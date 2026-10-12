#!/usr/bin/env node
/**
 * Preflight for `npm run tauri dev`.
 *
 * The desktop app's daemon auto-start re-executes the `service-orchestrator`
 * binary (`SERVICE_ORCHESTRATOR_DAEMON_BIN`, see
 * crates/client/src/socket.rs). That binary lives in the Cargo workspace at
 * the repository root and may not have been built yet on a fresh checkout, so
 * build it (incrementally) before Tauri starts. This keeps the Task 015
 * auto-start mechanism authoritative: the GUI never spawns daemons itself.
 */
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..', '..');
const manifest = join(repoRoot, 'Cargo.toml');

console.log('[desktop] building the service-orchestrator binary (daemon auto-start prerequisite)…');

const result = spawnSync(
  'cargo',
  ['build', '--manifest-path', manifest, '--bin', 'service-orchestrator'],
  { stdio: 'inherit', cwd: repoRoot },
);

if (result.error) {
  console.error(`[desktop] could not run cargo: ${result.error.message}`);
  console.error('[desktop] install a Rust toolchain (https://rustup.rs) and try again.');
  process.exit(1);
}
if (result.status !== 0) {
  console.error('[desktop] `cargo build --bin service-orchestrator` failed; fix the build and retry.');
  process.exit(result.status ?? 1);
}
