# ADR 009: The Library Manager

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

Shared libraries are built once and consumed by many services. A developer who
changes a library wants to build and install it locally and then build a
consumer against that local artifact — without pushing to a remote and without
silently changing what the consumer requests. ADR 001 and ADR 002 made `Library`
a first-class domain entity; ADR 005 established the task engine; ADR 006 the
process manager; ADR 007 the Git adapter; ADR 008 the Build Manager and its
provider abstraction.

The amendment in `tasks/009.md` shaped this decision with hard requirements:

- a Library Manager that **orchestrates existing capabilities** and does not
  directly execute shell commands;
- Maven is the first supported artifact ecosystem, but the architecture must
  remain open to Gradle;
- a Maven artifact is identified by its **coordinate**, never by repository name;
- do not hard-code `~/.m2/repository`; respect the **effective** Maven
  configuration and do not build a custom resolver;
- model reactor (multi-module) projects, and document the MVP limitation;
- dependency-aware ordering using the existing engine and graph, not a second
  scheduler;
- explicit, conservative version alignment (Aligned / Mismatch / Unknown /
  Unresolved) that never guesses;
- no naive POM rewriting; any automatic modification must be opt-in, dry-run,
  reversible, authorized, and tested — prefer deferring it;
- coordinate operations at library/artifact scope without a global lock;
- reuse the build capabilities, and return explicit errors for unsupported ones.

## Decision

Implement the Library Manager as **application-layer orchestration plus one new
port**, not as a new infrastructure crate.

```text
TUI / MCP / CLI
       │
       ▼
 Application ── Library Manager (commands + queries)
       │
       ├── Git tasks ──────────> Git adapter
       ├── Build tasks ────────> Build Manager ──> provider ──> Process Manager
       └── ArtifactVerification ─> ArtifactInspector (Maven, in `build`)
```

### Reusing the Build Manager, not reimplementing it

The Library Manager asks the Build Manager for **build-system-neutral**
capabilities (`Build`, `PublishLocalArtifact`), never for Maven. `install_library`
is `PublishLocalArtifact`; Maven performs it with its `install` phase. When a
library's build system cannot publish a local artifact, the request fails with
`BuildProviderUnsupported` rather than being approximated. No Maven CLI is
invoked inside the Library Manager, and no JAR is copied by hand.

### A new port for artifact inspection, implemented by the build adapter

Verification and version alignment are the only things the Library Manager needs
that are genuinely Maven-specific. Rather than leak Maven into the application,
the application declares:

- `ArtifactInspector` — `locate(ResolvedArtifact)` and
  `dependency_version(ResolvedDependency)`, both read-only, returning structured
  values;
- `ResolvedArtifact` — workspace + library + module path + coordinate, which
  round-trips through task parameters;
- `ArtifactPresence`, `DependencyVersionInspection`, and a coded
  `ArtifactInspectionError`.

The `build` crate implements the port as `MavenArtifactInspector`. This mirrors
the `GitManager` port (ADR 007): the application describes *what* to inspect and
the adapter knows *how*. A `NullArtifactInspector` keeps the application
constructible and its existing tests unchanged when no inspector is wired;
`Application::with_artifact_inspector` wires the real one, exactly like
`with_git_manager`.

### The effective local repository, discovered not assumed

`MavenArtifactInspector` does not hard-code `~/.m2/repository`. A
`LocalRepositoryProvider` seam resolves the effective value — the production
`MavenLocalRepositoryProvider` runs
`mvn -B -q -DforceStdout help:evaluate -Dexpression=settings.localRepository`
through the same `BuildRunner` every build uses, caches per module, and fails
explicitly (`ARTIFACT_LOCAL_REPOSITORY_UNKNOWN`) when it cannot. `FixedLocalRepository`
serves tests. Artifact presence is then a filesystem check at the standard Maven
layout path. There is no custom dependency resolver.

### Version alignment is read-only and conservative

Installing an artifact does not change the version a consumer requests, so
alignment is explicit. Consumer versions are read from a small, hand-written,
conservative POM reader: a literal version or a `${property}` defined in the same
POM yields `Declared`; a direct dependency without a version falls back to a
matching `dependencyManagement` entry; a property, parent POM, or imported BOM
the reader cannot resolve yields `Unknown`; an absent artifact with a parent or
imported BOM present yields `Unknown`; otherwise `NotDeclared`. The application
maps `Declared`+equal → `Aligned`, `Declared`+different → `Mismatch`, `Unknown`
→ `Unknown`, `NotDeclared` → `Unresolved`. No XML crate is added and nothing is
rewritten.

### Ordering and coordination without new machinery

The transitive closure of the selected libraries is ordered dependency-first by
a topological sort in the application layer, because the domain exposes only
crate-private cycle detection and a competing public graph was undesirable. Order
is expressed as **workflow-step dependencies**, so the task engine remains the
only scheduler. Two writers over the same module are serialized by
`ArtifactCoordinator`, a keyed async write lock inside the Build Manager keyed on
the module directory — no global lock, and reactor modules that share a directory
are covered.

### Minimal, additive domain and configuration changes

`Library` gains an additive `artifacts: Vec<ArtifactCoordinate>`;
`ArtifactCoordinate` gains optional `packaging`/`classifier` (defaulted, so
existing configuration is unaffected); `Library::produced_artifacts()` returns
them primary-first. `BuildOperation` already had `PublishLocalArtifact`.
`TaskKind` gains `LibraryInstall` (mapped to `PublishLocalArtifact`) and
`ArtifactVerification`; `WorkflowKind` gains `PrepareLibrary`/`PrepareLibraries`.
Existing variants and behaviour are unchanged.

## Alternatives considered

1. **A new `library` infrastructure crate with its own Maven execution.**
   Rejected: it would duplicate the Build Manager and the single-process-owner
   rule. The Library Manager is orchestration, and orchestration belongs in the
   application core.

2. **Identify artifacts by repository name.**
   Rejected (`tasks/009.md` §4): a repository may produce several artifacts and an
   artifact may be produced by a differently named module. Identity is the
   coordinate.

3. **Copy JARs into the local repository directly.**
   Rejected: it bypasses the build provider, ignores the effective repository
   layout, and can corrupt metadata. Installation is `PublishLocalArtifact`.

4. **Hard-code `~/.m2/repository`.**
   Rejected: it is frequently overridden. The effective location is discovered
   through Maven and cached, and a failure is explicit.

5. **Add a general Maven dependency resolver.**
   Rejected as scope and safety: resolving a full graph would require network
   access, credentials, and a large surface with no requirement. The Library
   Manager only checks presence and reads declared versions.

6. **Rewrite POMs (or run `versions:set`) to align versions.**
   Rejected for this task (`tasks/009.md` §13): automatic modification must be
   opt-in, dry-run, reversible, authorized, and tested. The safer, useful half —
   a read-only alignment plan — is shipped, and modification is deferred.

7. **Put version alignment (and verification) in the Prepare workflow.**
   Rejected: a mutating workflow should not block on a read-only inspection step.
   Alignment is its own query.

8. **A second scheduler for dependency-ordered library builds.**
   Rejected (`tasks/009.md` §10): the engine's dependency-aware scheduler already
   exists. Ordering is expressed as step dependencies.

9. **A global build lock to protect shared artifact state.**
   Rejected (`tasks/009.md` §23): it would serialize unrelated builds. A keyed
   write lock scoped to the module directory removes the specific hazard.

10. **Implement `GradleBuildProvider` now.**
    Rejected (`tasks/009.md` §31): Gradle publishing differs from Maven's and is
    out of scope. The provider abstraction and validation already accommodate it
    later.

## Consequences

### Positive

- One orchestration path serves the TUI and MCP: all library behaviour lives in
  the application core and reaches adapters only through the engine.
- The application has no Maven or POM knowledge; the build adapter owns it behind
  a port, and a null inspector keeps existing wiring and tests working.
- No hard-coded repository path, no custom resolver, and no unsafe POM
  manipulation.
- Multi-artifact (reactor) libraries are modelled without pretending they have a
  single output.
- Version alignment is explicit and honest: it reports `Unknown` rather than
  guessing.
- Coordination is scoped, so unrelated builds stay parallel.
- Gradle support is a localized future change: implement the provider, register
  it, extend validation, add tests — the Library Manager is unchanged.

### Negative / trade-offs

- The `build` adapter now implements a second application port
  (`ArtifactInspector`) in addition to being the `TaskExecutor`; the
  `build -> execution`/`build -> process` exceptions (ADR 008) apply unchanged.
- The conservative POM reader cannot resolve parent POMs, imported BOMs, or
  multi-hop properties, so those consumers report `Unknown`. This is a
  deliberate loss of precision in exchange for never guessing.
- The MVP verifies/installs/aligns on the **primary** coordinate; additional
  reactor artifacts are modelled and listed but not individually verified.
- Ordering is computed in the application rather than the domain because the
  domain keeps cycle detection crate-private; a public ordering API would let
  this move down later.
- A `.m2` probe runs Maven once per module (then cached), so the first
  verification in a module pays a Maven invocation.

## Follow-up

- Implement optional, dry-run, reversible version-override workflows when the
  requirements for safe POM modification are met (`tasks/009.md` §13, §15).
- Add per-additional-artifact verification for reactor libraries.
- Add `GradleBuildProvider` when a Gradle library appears.
- Surface library status, dependents, and alignment through the TUI and MCP
  strictly via the application use cases.
- Consider a public, reusable dependency-order API in the domain so ordering
  moves out of the application.
