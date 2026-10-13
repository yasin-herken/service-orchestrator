# Libraries

A **library** is a shared buildable artifact consumed by services: for example a
`common-core` JAR. Service Orchestrator treats libraries as first-class entities,
separate from services, so it can build them first, install them locally, verify
they are present, and report whether consumers agree on the version. The design
is in [`library-manager.md`](library-manager.md).

## 1. Library concepts

- A library is identified by its **artifact coordinate**
  (`group:artifact:version`), never by repository name.
- A library may produce a primary coordinate plus additional reactor artifacts.
- A library may depend only on **other libraries**, never on services.
- Libraries have no `health_checks`, `ports`, or `capabilities`.

```toml
[[libraries]]
id = "common-core"
name = "common-core"
workspace = "acme"
path = "libraries/common-core"
default_branch = "main"
type = "maven"                  # defaults to maven

[libraries.repository]
url = "git@github.com:acme/common-core.git"

[libraries.commands]
build = "./mvnw clean install"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "common-core"
version = "1.8.0-SNAPSHOT"
```

A `maven` library must declare artifact metadata and a build command.

## 2. Local installation

Installing a library means running its build system's *publish-local* operation —
for Maven, the `install` phase, which writes the artifact to the effective local
repository (`~/.m2/repository`, discovered through Maven, never assumed).

- **MCP** — `install_library { libraryId }`.
- **TUI** — Libraries screen, `i` on the current library.
- **GUI** — Libraries screen, Install.

The manager never copies a JAR by hand. A library whose build system cannot
publish a local artifact fails with `BUILD_PROVIDER_UNSUPPORTED`.

## 3. Artifact verification

`verify_library_artifact` checks whether a library's artifact is present in the
effective local repository and where it is.

- **MCP** — `verify_library_artifact { libraryId }`.
- **TUI** — Libraries screen, `v`.

Verification is read-only. A missing artifact is reported as `ARTIFACT_NOT_FOUND`
— it is not hidden and not guessed.

## 4. Library dependencies and consumer discovery

- **Library dependencies** order builds dependency-first. Selected libraries are
  built in dependency order using the engine's own scheduling; there is no second
  scheduler and no global lock.
- **Consumer discovery** (`find_library_dependents`) lists the services and
  libraries that consume a given library — useful before a breaking change.

## 5. Version alignment

`inspect_library_version_alignment` reports, **read-only**, whether each consumer
agrees with a library's configured artifact version. Each consumer is classified
as:

- **Aligned** — the consumer references the same version;
- **Mismatch** — a different version;
- **Unknown** — the reference could not be read;
- **Unresolved** — the version could not be resolved reliably.

Misalignment is **reported, never fixed**: no POM is rewritten and no version is
changed. This is deliberate
([`security.md`](security.md) §6, [`known-limitations.md`](known-limitations.md)).

## 6. Prepare a library

`prepare_library` composes the whole flow — synchronize, build, install, and
verify — as one workflow, and `prepare_selected_libraries` does it for several
libraries **in dependency order**.

- **MCP** — `prepare_library { libraryId, sync?, branch?, build?, install?, verify? }`
  and `prepare_selected_libraries { workspaceId, libraryIds, … }`.
- **TUI** — Libraries screen, `p`.
- **GUI** — Libraries screen, Prepare (with an options dialog).

Options default conservatively; enabling `sync` checks out the configured branch
before building.

## 7. Local-development workflow

You can build against a **locally modified** library without checking it out:

```text
prepare_library { libraryId: "common-core", sync: false, build: true, install: true, verify: true }
```

With `sync = false`, no checkout is scheduled, so your local library working tree
(including uncommitted changes) is left untouched. The consuming service is then
built against whatever is installed locally.

> **Important:** installing a library locally does **not** change the version any
> consumer depends on. If a service pins `common-core:1.7.0` and you install a
> `1.8.0-SNAPSHOT`, the service still resolves `1.7.0`. Aligning versions is a
> deliberate edit to the consumer's build file, which Service Orchestrator does
> not perform. Use `inspect_library_version_alignment` to see the gap.

## 8. TUI and GUI summary

| Operation | TUI | GUI | MCP |
| --- | --- | --- | --- |
| Build | `b` | Build | `build_library` |
| Install | `i` | Install | `install_library` |
| Prepare | `p` | Prepare | `prepare_library` / `prepare_selected_libraries` |
| Sync | `y` | Sync | (via prepare) |
| Verify artifact | `v` | Verify | `verify_library_artifact` |
| Version alignment | `a` | Alignment | `inspect_library_version_alignment` |
| Refresh status | `r` | — | `get_library_status` |
| Dependents | (detail) | Dependents | `find_library_dependents` |

Inspection columns are populated only after you ask for them; the screens never
scan artifacts or repositories on their own.

## 9. Common library errors

| Code | Meaning | Action |
| --- | --- | --- |
| `ARTIFACT_NOT_FOUND` | The artifact is not in the local repository. | Install or prepare the library. |
| `ARTIFACT_VERSION_MISMATCH` | A consumer references a different version. | Align the consumer's version manually. |
| `BUILD_PROVIDER_UNSUPPORTED` | The build system cannot publish locally. | Use Maven/npm. |
| `LIBRARY_BUILD_FAILED` | The library build failed. | Read the build logs. |
| `VERSION_INSPECTION_UNSUPPORTED` | Version inspection is not supported for this library's system. | Inspect manually. |
| `ARTIFACT_LOCAL_REPOSITORY_UNKNOWN` | The effective local repository could not be discovered. | Ensure Maven is installed and configured. |

## 10. Limitations

- Automatic version alignment (rewriting consumer build files) is intentionally
  not implemented.
- The TUI/GUI report alignment but offer no one-click upgrade.
