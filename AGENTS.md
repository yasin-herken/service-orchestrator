# AGENTS.md

## Service Orchestrator

This repository contains **Service Orchestrator**, a macOS-native developer environment orchestration platform written in Rust.

Service Orchestrator manages local development workflows for multiple repositories and microservices through a single application.

The application provides:

- A Ratatui-based TUI for human developers
- An MCP server for AI agents
- A shared application core used by both interfaces
- Git management
- Build orchestration
- Library management
- Liquibase execution
- Local process management
- Health checks
- Logs
- Dependency-aware workflows
- Profiles and service groups

---

# 1. Core Architectural Principle

The most important rule in this repository is:

> **The application core must never depend on the TUI or MCP.**

Ratatui and MCP are interfaces/adapters around the same application core.

The intended architecture is:

```text
                  Human Developer
                        │
                     Ratatui
                        │
                        ▼
               ┌─────────────────┐
               │ Application Core│
               └────────┬────────┘
                        │
          ┌─────────────┼─────────────┐
          │             │             │
          ▼             ▼             ▼
         Git          Build        Processes
          │             │             │
          ▼             ▼             ▼
        GitHub        Maven        Java/Node


                    AI Agent
                        │
                       MCP
                        │
                        ▼
               ┌─────────────────┐
               │ Application Core│
               └─────────────────┘
```

Never implement separate business logic for TUI and MCP.

For example, this is correct:

```text
TUI -> StartService use case -> Task Engine
MCP -> StartService use case -> Task Engine
```

This is incorrect:

```text
TUI -> custom start logic

MCP -> different start logic
```

There must be one source of truth for application behavior.

---

# 2. Product Philosophy

Service Orchestrator is primarily a **developer orchestration platform**, not an AI application.

AI is an optional interface.

The application must remain fully functional without:

- An LLM
- An MCP client
- Internet access
- A remote backend

A developer must be able to use:

```bash
service-orchestrator tui
```

without involving any LLM.

AI-assisted usage is optional:

```bash
service-orchestrator mcp
```

This distinction must be preserved throughout the architecture.

---

# 3. Primary Goals

The project exists to reduce repetitive developer environment preparation.

The system should allow developers to:

- Select required services
- Synchronize repositories
- Checkout branches
- Build shared libraries
- Run Liquibase migrations
- Build applications
- Start applications
- Stop applications
- Restart applications
- Inspect logs
- Check health
- Execute workflows
- Manage service groups
- Use development profiles

The system should also allow AI agents to perform the same operations through MCP.

---

# 4. Existing Service Technology

The services managed by this application currently use:

## Backend

- Java
- Spring Boot 3
- Maven

## Frontend

- React 16
- Node.js 20.19.4

## Database Migration

- Liquibase

## Source Control

- Git

## Deployment

- Kubernetes

Do not assume that every future service will use the same stack.

The Service Orchestrator must be extensible.

---

# 5. Architecture Layers

Keep the following conceptual layers separate.

## Domain

Pure business concepts and rules.

Examples:

```text
Service
Workspace
Repository
Library
Profile
ServiceGroup
Task
Workflow
Dependency
Runtime
HealthStatus
GitState
ProcessState
Permission
```

Domain code should not depend on:

- Ratatui
- MCP
- OS-specific process APIs
- Git implementation details
- filesystem implementation details
- terminal rendering

---

## Application

Application use cases and orchestration.

Examples:

```text
ListServices
GetServiceStatus
SyncService
BuildService
BuildLibrary
StartService
StopService
RestartService
RunLiquibase
PrepareEnvironment
StartProfile
StopProfile
GetLogs
HealthCheck
```

Application code coordinates domain and infrastructure services.

---

## Infrastructure

External integrations.

Examples:

```text
Git
Filesystem
Process execution
Maven
Node/npm
Liquibase
HTTP
Health checks
Logging
Environment discovery
```

Infrastructure implementations should be replaceable where practical.

---

## Interfaces

External interfaces:

```text
TUI
MCP
CLI
Future GUI
Future API
```

Interfaces should call application use cases instead of implementing business behavior.

---

# 6. Project Structure

Prefer a modular Cargo workspace.

A possible structure is:

```text
service-orchestrator/
├── Cargo.toml
│
├── crates/
│   ├── domain/
│   ├── application/
│   ├── config/
│   ├── execution/
│   ├── git/
│   ├── process/
│   ├── build/
│   ├── liquibase/
│   ├── health/
│   ├── logging/
│   ├── policy/
│   ├── mcp/
│   └── tui/
│
├── docs/
│   ├── architecture.md
│   ├── configuration.md
│   ├── workflows.md
│   ├── mcp.md
│   ├── security.md
│   └── adr/
│
├── tests/
│
└── AGENTS.md
```

This is guidance, not a rigid requirement.

If a different structure is technically better, use it and document the reasoning.

---

# 7. Rust Engineering Rules

Write idiomatic, maintainable Rust.

Prefer:

- Strong types
- Explicit ownership
- Small modules
- Small interfaces
- Composition
- Dependency injection where useful
- Trait abstractions at real boundaries
- `Result`-based error handling
- Structured errors
- Async execution where appropriate

Avoid:

- Excessive global state
- Excessive `Arc<Mutex<...>>`
- Unnecessary traits
- Giant enums used as universal state containers
- Giant service classes/modules
- Monolithic `main.rs`
- UI-driven architecture
- Unnecessary macros
- Premature plugin frameworks

Do not introduce abstractions without a concrete reason.

---

# 8. Async and Concurrency

Use asynchronous execution where operations involve:

- Processes
- I/O
- Network requests
- Multiple independent tasks
- Logs
- Health checks

Prefer Tokio unless a strong reason exists to use something else.

The task engine must support:

```text
queued
running
succeeded
failed
cancelled
skipped
blocked
```

Independent tasks may execute concurrently.

Dependency-constrained tasks must wait for their dependencies.

---

# 9. Task Engine

All meaningful operations must be represented as tasks.

Examples:

```text
GitFetchTask
GitCheckoutTask
GitPullTask
MavenBuildTask
MavenInstallTask
NpmInstallTask
NpmBuildTask
LiquibaseTask
StartProcessTask
StopProcessTask
HealthCheckTask
```

Long-running operations should return task IDs.

Example:

```json
{
  "taskId": "12345",
  "status": "running"
}
```

Do not block an interface unnecessarily while long-running operations execute.

The TUI must be able to display task progress.

MCP must be able to query task status.

---

# 10. Workflow Engine

Workflows are compositions of tasks.

Example:

```text
PrepareEnvironment
    │
    ├── Validate
    ├── Git Sync
    ├── Build Libraries
    ├── Liquibase
    ├── Build Services
    ├── Start Services
    └── Health Checks
```

The workflow engine should support:

- Dependencies
- Parallel execution
- Sequential execution
- Conditions
- Retry
- Timeout
- Cancellation
- Failure propagation
- Progress reporting

Avoid embedding workflow logic inside interface code.

---

# 11. Service Executors

Service-specific behavior must be isolated.

Possible abstractions:

```rust
trait ServiceExecutor {
    fn validate(&self, service: &Service) -> Result<()>;
    fn build(&self, service: &Service) -> TaskDefinition;
    fn start(&self, service: &Service) -> TaskDefinition;
    fn stop(&self, service: &Service) -> TaskDefinition;
}
```

Potential implementations:

```text
JavaServiceExecutor
NodeServiceExecutor
LibraryExecutor
```

Future implementations should be possible:

```text
PythonServiceExecutor
GoServiceExecutor
DockerExecutor
KubernetesExecutor
```

Do not hard-code service-specific logic into generic orchestration code.

---

# 12. Configuration-Driven Design

Services must not be hard-coded into Rust source files.

Use external configuration.

Configuration should define things such as:

- Workspace
- Services
- Service type
- Repository
- Local path
- Default branch
- Runtime
- Build command
- Start command
- Stop behavior
- Environment variables
- Ports
- Health checks
- Dependencies
- Groups
- Libraries

Prefer a versioned TOML configuration unless there is a strong reason to use another format.

Example:

```toml
config_version = 1

[workspace]
name = "project-x"
root = "~/Projects/project-x"

[[services]]
name = "auth-service"
type = "backend"
path = "backend/auth-service"
default_branch = "master"
port = 8081

[services.runtime]
type = "java"
version = "21"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[services.liquibase]
enabled = true
command = "./mvnw liquibase:update"
health_url = "http://localhost:8081/actuator/health"
```

---

# 13. Do Not Hard-Code `master`

The current default branch may be `master`, but it must come from configuration.

Never write logic equivalent to:

```rust
checkout("master")
```

inside core business logic.

Instead use configured branch information.

Future configurations may use:

```text
main
master
develop
release/*
feature/*
```

---

# 14. Git Rules

Git operations should be implemented behind a Git abstraction.

Supported operations should include:

```text
status
fetch
pull
checkout
branch list
diff
reset
clean
```

Before destructive operations, validate Git state.

If local changes exist, do not silently discard them.

The user must be clearly informed before:

```text
git reset --hard
git clean
discard local changes
```

MCP requests must be subject to the exact same policies as TUI requests.

---

# 15. Process Management

Local processes are first-class resources.

The process manager must track:

- PID
- Service
- Command
- Start time
- Exit code
- State
- Port
- Child processes
- Logs

Handle:

- Graceful stop
- Forced termination
- Cancellation
- Process failure
- Process cleanup

Avoid orphaned child processes.

A process being alive should not automatically mean the service is healthy.

---

# 16. Health Checks

Health checks are configurable.

Possible checks:

```text
Process
Port
HTTP
Spring Boot actuator
Docker health
Kubernetes readiness
Kubernetes liveness
```

For Spring Boot services, an actuator endpoint such as:

```text
/actuator/health
```

may be used.

Do not assume all services expose the same endpoint.

---

# 17. Library Management

Libraries are first-class entities.

Typical workflow:

```text
Library
    ↓
Git sync
    ↓
Checkout configured branch
    ↓
mvn clean install
    ↓
Verify artifact
    ↓
Build dependent services
```

Library version information must be configurable.

Example:

```text
com.company:common-core:1.8.0-SNAPSHOT
```

The system should eventually understand library-to-service dependency relationships.

---

# 18. Dependency Graph

Model service dependencies explicitly where possible.

Example:

```text
common-core
    ├── auth-service
    ├── user-service
    └── payment-service

auth-service
    └── user-service
```

Use dependencies to determine safe execution order.

Do not assume that service order is alphabetical or configuration order.

---

# 19. Service Groups

Support logical groups.

Example:

```text
Customer
    auth-service
    user-service
    customer-service

Payment
    payment-service
    billing-service

Frontend
    web-ui
    admin-ui
```

Group actions should reuse the same application-level workflows.

---

# 20. Profiles

Profiles represent commonly used developer environments.

Examples:

```text
backend-development
payment-development
customer-development
full-stack
```

Profiles should reference configured services rather than duplicate service configuration.

---

# 21. MCP Principles

MCP is an interface to the application core.

It is NOT the business layer.

MCP handlers must be thin adapters.

Correct:

```text
MCP Tool
    ↓
Application Use Case
    ↓
Task Engine
    ↓
Infrastructure
```

Incorrect:

```text
MCP Tool
    ↓
Git command
    ↓
Maven command
    ↓
Process handling
```

Do not duplicate application logic inside MCP handlers.

---

# 22. MCP Transport

The primary local MCP transport should be stdio.

The binary should support:

```bash
service-orchestrator mcp
```

An MCP-compatible client should be able to launch the process directly.

Do not introduce an external HTTP backend only for MCP.

Other MCP transports may be introduced later through a transport abstraction.

---

# 23. MCP Tool Design

Prefer explicit domain tools.

Good:

```text
list_services
get_service_status
git_status
git_fetch
git_pull
git_checkout
build_service
build_library
run_liquibase
start_service
stop_service
restart_service
prepare_environment
start_profile
stop_profile
get_task
cancel_task
get_logs
health_check
```

Avoid using this as the main AI API:

```text
run_shell(command)
```

The AI should not need to construct arbitrary shell commands for normal developer workflows.

---

# 24. MCP Tool Granularity

Use three conceptual levels.

## Primitive

```text
git_fetch
git_checkout
git_pull
maven_build
liquibase_update
process_start
process_stop
```

## Service

```text
sync_service
build_service
start_service
restart_service
```

## Workflow

```text
prepare_environment
start_profile
build_workspace
sync_workspace
```

AI agents should generally use service-level and workflow-level operations.

Primitive tools exist for diagnostics and advanced control.

---

# 25. MCP Resources

Use resources for read-oriented structured state.

Examples:

```text
workspace://project-x
workspace://project-x/services
service://auth-service
service://payment-service
task://12345
logs://auth-service
```

Do not force AI agents to parse terminal-formatted output when structured data is available.

---

# 26. MCP Prompts

Where useful, expose standardized developer workflows.

Examples:

```text
prepare-backend-environment
prepare-payment-environment
prepare-full-stack-environment
debug-service
restart-failed-services
sync-workspace
build-required-libraries
```

Do not put business logic into MCP prompts.

Prompts should guide the AI toward the correct application capabilities.

---

# 27. MCP Schemas

MCP tools must have strict input schemas.

Validate:

- Service names
- Workspace names
- Profile names
- Branch names
- Task IDs
- Paths
- Boolean workflow options
- Runtime configuration

Do not accept ambiguous free-form strings when an enum or structured input is possible.

---

# 28. MCP Error Handling

Return structured errors.

Example:

```json
{
  "success": false,
  "error": {
    "code": "MAVEN_BUILD_FAILED",
    "service": "auth-service",
    "taskId": "12345",
    "exitCode": 1,
    "message": "Maven build failed"
  }
}
```

AI agents should not need to scrape raw terminal output to understand the high-level failure.

Detailed logs may be available separately.

---

# 29. MCP Permissions

All operations must pass through the same policy engine.

Classify operations as:

```text
READ
SAFE_WRITE
DESTRUCTIVE
```

Example:

```text
READ
----
list_services
get_status
get_logs
git_status
health_check

SAFE_WRITE
----------
git_fetch
git_pull
build_service
build_library
run_liquibase
start_service
stop_service

DESTRUCTIVE
-----------
git_reset
git_clean
discard_local_changes
delete_workspace
force_stop
```

The MCP interface must never bypass these policies.

---

# 30. No Implicit Destructive Actions

AI agents are not allowed to silently perform destructive operations.

Examples:

```text
git reset --hard
git clean
delete repository
discard changes
delete workspace
```

must require explicit authorization/confirmation according to the policy model.

The same rule must apply to TUI operations.

---

# 31. Secrets

Never expose secrets through:

- MCP
- TUI
- logs
- task results
- errors
- configuration output

Do not store secrets in standard configuration files.

Environment variables containing credentials must be redacted when displayed.

---

# 32. Shell Command Execution

Prefer structured process execution:

```rust
Command::new("mvn")
    .args(["clean", "install"])
```

over:

```text
sh -c "mvn clean install"
```

Do not pass untrusted values directly into shell commands.

Validate all paths and arguments.

Raw shell execution may only be used where required by a real project-specific workflow.

---

# 33. TUI Rules

Ratatui widgets must remain presentation-oriented.

Do not put:

- Git logic
- Maven logic
- process execution
- business rules
- workflow orchestration

inside widgets.

TUI should:

1. Read application state
2. Render state
3. Create commands/intents
4. Dispatch commands to the application layer

Example:

```text
User presses Start
        ↓
TUI creates StartService command
        ↓
Application layer
        ↓
Task Engine
```

---

# 34. TUI UX

The TUI should prioritize:

- Keyboard-first interaction
- Multi-selection
- Search/filter
- Clear status indicators
- Realtime task progress
- Realtime logs
- Confirmation dialogs
- Actionable errors

The interface should remain usable with dozens or hundreds of services.

Do not assume a fixed number of rows.

---

# 35. Logs

Use structured internal logging.

Prefer `tracing` for application logs.

Each task should have associated logs.

Each service should have its own log stream.

Logs should support:

- Realtime viewing
- Historical viewing
- Filtering
- Bounded retrieval
- Severity
- Timestamp

Avoid returning unbounded log content through MCP.

---

# 36. Configuration Storage

A default configuration location may be:

```text
~/.service-orchestrator/
    config.toml
    workspaces/
    profiles/
    logs/
```

Configuration schema should be versioned.

Example:

```toml
config_version = 1
```

Future versions must be able to migrate old configuration safely.

---

# 37. macOS

The application targets macOS.

At minimum:

- Apple Silicon support
- Native execution
- Single binary
- Correct PATH/runtime detection

Prefer supporting Intel macOS where feasible.

Environment discovery should detect:

```text
git
java
mvn
mvnw
node
npm
kubectl
```

Do not assume tools are installed globally.

Use project-local wrappers where appropriate.

---

# 38. Runtime Detection

The application should provide a diagnostic command:

```bash
service-orchestrator doctor
```

It should validate:

```text
Git
Java
Maven
Node
npm
kubectl
workspace paths
configuration
ports
```

Output should be actionable.

---

# 39. Future Kubernetes Support

Kubernetes is not an MVP requirement.

However, the architecture should make future support possible.

Potential runtime abstraction:

```text
RuntimeTarget
    ├── Local
    ├── Docker
    └── Kubernetes
```

Do not introduce Kubernetes-specific dependencies into core domain logic.

---

# 40. Performance

The system should be efficient enough to handle approximately:

```text
40 services
```

and eventually:

```text
100+
100-200
```

Potential performance concerns include:

- Process count
- Git operations
- Parallel builds
- Log volume
- TUI rendering
- Dependency graph size
- MCP response size
- Configuration loading

Avoid scanning every repository continuously unless necessary.

Prefer event-driven or on-demand status checks.

---

# 41. Resource Management

The scheduler must enforce configurable concurrency.

Example:

```toml
[execution]
max_parallel_tasks = 4
```

Do not start 40 heavy builds simultaneously by default.

Future support may include:

- CPU-aware scheduling
- Memory-aware scheduling
- Priorities
- Dynamic concurrency

---

# 42. Error Handling

Use structured errors.

Prefer domain/application-specific error categories.

Examples:

```text
ConfigurationError
GitError
BuildError
ProcessError
LiquibaseError
HealthCheckError
PermissionError
TaskError
WorkspaceError
```

Use `thiserror` or another appropriate error library for typed errors.

Use `anyhow` at application boundaries when useful.

Do not discard useful context.

---

# 43. Testing

Every significant feature must include appropriate tests.

## Unit tests

Test:

- Configuration parsing
- Configuration validation
- Domain rules
- Task states
- Scheduler logic
- Dependency resolution
- Permission policies
- Command construction
- MCP schema validation

## Integration tests

Test:

- Git fixture repositories
- Process lifecycle
- Maven fixture projects
- Health checks
- MCP tool invocation
- Workflow execution

Use deterministic fixtures.

Do not depend on real developer repositories.

---

# 44. MCP Integration Tests

MCP integration tests should verify the entire adapter flow:

```text
Tool request
    ↓
MCP adapter
    ↓
Validation
    ↓
Permission policy
    ↓
Application use case
    ↓
Task Engine
    ↓
Infrastructure
    ↓
Structured MCP response
```

Do not test only the MCP handler in isolation.

---

# 45. Documentation

Update documentation whenever architecture or user-facing behavior changes.

Important documentation:

```text
README.md
docs/architecture.md
docs/configuration.md
docs/workflows.md
docs/mcp.md
docs/security.md
docs/getting-started.md
docs/adr/*
```

MCP documentation must describe:

- Connection model
- stdio usage
- Available tools
- Resources
- Prompts
- Permissions
- Example AI workflows

---

# 46. ADRs

Create an ADR for significant architectural decisions.

Examples:

```text
001-core-architecture.md
002-task-engine.md
003-process-management.md
004-git-integration.md
005-configuration.md
006-mcp-integration.md
007-mcp-security.md
008-concurrency.md
009-service-executors.md
```

Each ADR should document:

- Context
- Decision
- Alternatives
- Reasoning
- Consequences

---

# 47. Development Workflow

Before implementing a major feature:

1. Understand the current architecture.
2. Identify the correct layer.
3. Reuse existing abstractions.
4. Avoid duplicating logic.
5. Consider TUI and MCP implications.
6. Add tests.
7. Update documentation where necessary.

Do not make large architectural changes without first examining existing code.

---

# 48. Change Discipline

When modifying existing functionality:

- Preserve existing behavior unless change is intentional.
- Avoid unrelated refactoring.
- Keep commits logically scoped.
- Do not rewrite working components without justification.
- Prefer incremental improvements.

Do not introduce a new abstraction merely because it looks architecturally elegant.

Introduce abstractions when there is a real extension or separation requirement.

---

# 49. Build and Validation

Before considering an implementation complete, run the relevant checks.

At minimum, when applicable:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Use additional integration tests where necessary.

Do not ignore compiler or test failures.

---

# 50. No Fake Implementations

Do not claim a feature works if it is only
