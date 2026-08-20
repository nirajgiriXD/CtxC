# CtxC Architecture

> Cross-platform context optimization for AI agents.

## 1. Purpose

CtxC (Context Compiler) is a cross-platform, local-first context optimization engine for AI agents.

Its primary purpose is to reduce unnecessary context and token consumption while preserving the information required for an AI agent to correctly perform a task.

CtxC should work on:

- macOS
- Linux
- Windows

The core system must not require:

- Python
- Node.js
- Docker
- PostgreSQL
- Redis
- Cloud services
- A specific LLM provider

The basic CtxC CLI must be usable as a standalone native executable.

---

## 2. Core Philosophy

CtxC is not a generic text compressor.

The system should optimize context using the following hierarchy:

```text
Understand
    ↓
Select
    ↓
Transform
    ↓
Compress
    ↓
Budget
    ↓
Deliver
    ↓
Retrieve when necessary
```

The primary optimization objective is:

> Maximize useful information per token.

Do not optimize for token reduction at the expense of correctness.

A 10,000-token context containing the required information is better than a 2,000-token context that omits critical information.

---

## 3. Design Principles

### 3.1 Local First

CtxC should perform as much processing locally as possible.

Local processing provides:

- Privacy
- Low latency
- Offline operation
- Predictable costs
- No mandatory API keys

External AI models may be introduced as optional capabilities later.

### 3.2 Model Agnostic

The core must not depend on OpenAI, Anthropic, Google, or any other specific provider.

LLM integrations belong in adapters.

The core should operate on context independently of the destination model.

### 3.3 Cross Platform

Cross-platform support is a core requirement, not a later feature.

Avoid platform-specific assumptions throughout the core.

Platform-specific behavior must be isolated behind abstractions.

### 3.4 Deterministic First

Prefer deterministic optimization whenever possible.

Examples:

- Deduplication
- Structured output reduction
- AST analysis
- Dependency analysis
- JSON optimization
- Log filtering
- Relevance heuristics
- Token budgeting

AI/ML-based optimization should be an optional enhancement rather than a mandatory dependency.

### 3.5 Reversible Optimization

When information is removed from the immediate context, the original information should remain recoverable whenever practical.

Example:

```text
Optimized Context
      |
      +---- context://abc123
                   |
                   v
              Original Data
```

The agent should be able to retrieve omitted information when needed.

---

## 4. High-Level Architecture

CtxC has two entry surfaces (CLI and Dashboard) and one runtime (the daemon).

```text
                         CtxC
                          |
             +------------+------------+
             |                         |
            CLI                     Dashboard
             |                         |
             +------------+------------+
                          |
                       Daemon
                          |
       +------------------+------------------+
       |                  |                  |
   Project Manager      Metrics          HTTP API
       |                  |                  |
       v                  v                  |
   File Watcher       SQLite <---------------+
       |
       v
 Incremental Indexer
       |
       +----------------------+
       |                      |
       v                      v
 Code Intelligence       Context Store
       |                      |
       v                      |
 Context Graph                |
       |                      |
       +----------+-----------+
                  |
                  v
            Context Engine
                  |
       +----------+----------+
       |          |          |
    Retrieval   Ranking   Optimization
       |          |          |
       +----------+----------+
                  |
                  v
           Context Budget
                  |
                  v
          Optimized Context
                  |
                  v
               AI Agent
                  |
         +--------+--------+
         |        |        |
       Hooks     MCP     Proxy
```

The optimization pipeline itself remains unchanged and is reachable from either surface:

```text
   Acquisition        Intelligence        Storage
          |                 |                 |
          v                 v                 v
       Shell              AST              SQLite
       Files              Graph            Cache
       Git                Ranking           Memory
       APIs               Retrieval
       Logs
          |                 |
          +--------+--------+
                   |
                   v
            Context Router
                   |
                   v
             Context Ranker
                   |
                   v
          Content Optimizers
                   |
          +--------+--------+
          |        |        |
         Code     JSON     Text
          |        |        |
          +--------+--------+
                   |
                   v
            Context Budget
                   |
                   v
          Optimized Context
```

### 4.1 Operating Modes

CtxC supports two modes of operation. Both use the same engine.

**One-shot** — stateless, scriptable, no daemon required:

```bash
ctxc optimize build.log
git status | ctxc optimize --from "git status"
```

Useful for scripts, CI, pipes, debugging, and first-time usage.

**Managed** — the normal long-running mode:

```bash
ctxc project add ./project
ctxc start
```

CtxC then continuously observes registered projects and keeps their index, graph, and cache current as activity happens.

One-shot mode must always remain available. Managed mode is the default experience.

CtxC should feel like infrastructure, not like a command that must be invoked repeatedly.

---

## 5. Technology Stack

### Core Language

Use:

- Rust

Reasons:

- Native binaries
- Excellent cross-platform support
- Low memory usage
- Fast startup
- Strong filesystem/process APIs
- Strong concurrency model
- Suitable for CLI and daemon applications
- Suitable for long-running processes
- No runtime installation required

### Async Runtime

Use:

- Tokio

Use asynchronous execution for:

- Daemon
- HTTP server
- Proxy
- MCP
- Concurrent file processing
- Background indexing

Do not introduce async complexity into components that do not need it.

### CLI

Use:

- clap

The CLI should expose a stable command structure, and must never advertise a command that does not work: the command tree is the contract, not a wish list.

The tree is organised around what a user is doing rather than around which crate does it. Content commands (`analyze`, `optimize`, `compile`, `capture`, `retrieve`) act on one piece of material. Project commands (`index`, `graph`, `search`, `similar`, `project`) act on a registered or indexed directory. Lifecycle commands (`start`, `stop`, `daemon`) manage the runtime. `ctxc start` and `ctxc stop` are the user-facing aliases for daemon lifecycle management; `ctxc daemon` remains available for lower-level control and diagnostics.

The command surface as it exists today — every command, argument, and flag — is documented in [USAGE.md](USAGE.md#command-reference).

### HTTP

Use:

- Axum

The HTTP layer should be isolated from the core engine.

The core must not depend directly on HTTP.

### Serialization

Use:

- serde
- serde_json

Use JSON for public interoperability.

Internal high-performance formats may be introduced later.

### Configuration

Use:

- TOML

Default configuration location must be platform appropriate.

Do not hardcode Linux-specific paths.

Use platform-aware directories.

### Database

Use:

- SQLite

SQLite is the initial persistence layer.

Do not require:

- PostgreSQL
- MySQL
- Redis
- Docker

Use SQLite for:

- Context metadata
- Cached context
- Memory
- Graph relationships
- Index metadata
- Retrieval metadata

Use SQLite FTS5 for full-text search.

Vector search should remain optional and can be added later.

### Code Parsing

Use:

- Tree-sitter

Tree-sitter should provide deterministic code understanding.

Initial responsibilities:

- Language detection
- Symbol extraction
- Imports
- Exports
- Functions
- Classes
- References
- Basic relationships

Do not require an LLM to understand source-code structure.

---

## 6. Workspace Structure

Use a Rust Cargo workspace.

Recommended initial structure:

```text
ctxc/
│
├── crates/
│   ├── ctxc-cli/
│   ├── ctxc-core/
│   ├── ctxc-context/
│   ├── ctxc-engine/
│   ├── ctxc-optimizer/
│   ├── ctxc-parser/
│   ├── ctxc-graph/
│   ├── ctxc-store/
│   ├── ctxc-retrieval/
│   ├── ctxc-project/
│   ├── ctxc-watcher/
│   ├── ctxc-metrics/
│   ├── ctxc-daemon/
│   ├── ctxc-api/
│   ├── ctxc-proxy/
│   └── ctxc-mcp/
│
├── dashboard/
│   ├── src/
│   ├── index.html
│   ├── package.json
│   └── vite.config.ts
│
├── integrations/
│   ├── claude/
│   ├── codex/
│   ├── gemini/
│   ├── cursor/
│   ├── copilot/
│   ├── cline/
│   └── opencode/
│
├── tests/
│   ├── fixtures/
│   ├── integration/
│   └── benchmarks/
│
├── docs/
│
├── Cargo.toml
├── README.md
├── ARCHITECTURE.md
├── LICENSE
└── .github/
    └── workflows/
```

`dashboard/` is a build-time-only directory. Its compiled assets are embedded into the CtxC binary during release builds.

Node.js is required to build the dashboard, but never to run CtxC. A user who installs CtxC must not need Node, npm, or a separate web application install.

---

## 7. Crate Responsibilities

### ctxc-core

Contains foundational types and traits.

Must remain lightweight.

Responsibilities:

- Core data types
- Context identifiers
- Token budgets
- Context metadata
- Error types
- Shared traits

Example:

```rust
pub struct ContextId(String);

pub struct Context {
    pub id: ContextId,
    pub content: String,
    pub metadata: ContextMetadata,
}

pub struct ContextMetadata {
    pub source: ContextSource,
    pub content_type: ContentType,
    pub token_count: Option<usize>,
}
```

Do not put business logic here.

### Runtime crates

The runtime crates are described functionally later in this document:

| Crate | Section |
| --- | --- |
| `ctxc-project` | 25, 28 |
| `ctxc-watcher` | 26, 27 |
| `ctxc-metrics` | 29 |
| `ctxc-api` | 31 |
| `ctxc-daemon` | 24 |

Dependency direction must stay one-way. The engine must not depend on the watcher, the metrics subsystem, the API, or the dashboard. Those depend on the engine.

---

## 8. ctxc-context

Responsible for representing and manipulating context.

Responsibilities:

- Context ingestion
- Context normalization
- Context metadata
- Context fragments
- Context references
- Context serialization

Conceptually:

```text
Context
 ├── Fragment
 ├── Metadata
 ├── Source
 ├── Relationships
 └── Reference
```

---

## 9. ctxc-engine

The orchestration layer.

Responsibilities:

- Execute optimization pipelines
- Coordinate acquisition
- Coordinate analysis
- Coordinate retrieval
- Coordinate optimizers
- Enforce context budgets

The engine should not implement individual optimizers.

Instead:

```text
Engine
   |
   +-- Analyzer
   +-- Router
   +-- Ranker
   +-- Optimizer
   +-- Budgeter
   +-- Store
```

---

## 10. ctxc-optimizer

Contains content optimization implementations.

Use a plugin-like trait architecture.

Example:

```rust
pub trait ContextOptimizer {
    fn supports(&self, context: &Context) -> bool;

    fn optimize(
        &self,
        context: &Context,
        budget: Option<TokenBudget>,
    ) -> Result<OptimizedContext>;
}
```

Initial optimizers:

- CodeOptimizer
- JsonOptimizer
- TextOptimizer
- LogOptimizer
- MarkdownOptimizer
- TerminalOptimizer

Do not create one generic compression algorithm.

---

## 11. Content Router

The router determines which optimizer should handle a context.

```text
                    Context
                       |
                       v
                Content Router
                       |
       +---------------+---------------+
       |               |               |
      Code            JSON            Text
       |               |               |
       v               v               v
   CodeOptimizer  JsonOptimizer  TextOptimizer
```

Routing should use:

- MIME type
- File extension
- Detected language
- Structural analysis
- Content heuristics

The router should not call an LLM.

---

## 12. ctxc-parser

Responsible for deterministic parsing.

Initial support:

- TypeScript
- JavaScript
- Python
- Rust
- Go
- Java
- C
- C++
- PHP
- HTML
- CSS
- JSON
- Markdown

Language support should be modular.

Example:

```text
ParserRegistry
     |
     +-- TypeScriptParser
     +-- PythonParser
     +-- RustParser
     +-- GoParser
     +-- ...
```

---

## 13. Code Intelligence

Use Tree-sitter.

Extract:

```text
File
 ├── imports
 ├── exports
 ├── functions
 ├── classes
 ├── methods
 ├── variables
 └── references
```

Example:

```text
auth.ts
 |
 +-- imports → database.ts
 |
 +-- exports → authenticate()
 |
 +-- references → User
 |
 +-- referenced-by → api.ts
```

This information should be available to the retrieval layer.

---

## 14. ctxc-graph

Responsible for context relationships.

The graph must support explicit relationships first.

Example:

```text
auth.ts
    |
    +-- imports --> database.ts
    |
    +-- imports --> session.ts
    |
    +-- exports --> authenticate()
```

Relationship types should be extensible.

Initial relationship types:

- imports
- exports
- calls
- references
- extends
- implements
- contains
- depends_on
- tests
- documented_by
- generated_from

Do not require an LLM to create basic code relationships.

---

## 15. ctxc-store

Persistence layer.

SQLite database should store:

- contexts
- context_fragments
- files
- symbols
- relationships
- embeddings
- memories
- cache_entries
- optimization_results
- projects
- project_settings
- watch_events
- index_state
- metric_events
- metric_rollups
- activity_log

The database schema should be versioned through migrations.

Do not expose raw SQLite usage throughout the application.

Use repository interfaces:

```rust
trait ContextStore {
    fn save_context(...);
    fn get_context(...);
    fn delete_context(...);
}
```

This keeps storage replaceable.

---

## 16. Context Retrieval

Retrieval should be hybrid.

Use:

```text
                    Query
                      |
          +-----------+-----------+
          |                       |
          v                       v
    Keyword Search          Semantic Search
       FTS5                  Optional
          |                       |
          +-----------+-----------+
                      |
                      v
               Graph Expansion
                      |
                      v
                 Ranking
                      |
                      v
             Relevant Context
```

Semantic retrieval should be optional initially.

CtxC must function without an embedding model.

---

## 17. Context Ranking

Every candidate context fragment should receive a relevance score.

Possible factors:

- relevance
- recency
- dependency distance
- explicit user reference
- file importance
- symbol relationship
- task similarity
- source reliability

Example:

```text
score =
    semantic_similarity
  + graph_relevance
  + explicit_reference
  + recency
  + source_priority
```

The scoring system should be configurable.

Do not hardcode one universal scoring formula permanently.

---

## 18. Context Budgeting

Context budgeting is a first-class subsystem.

Input:

- Available Context
- Token Budget
- System Context
- Conversation
- Task

Output:

- Optimized Context

Example:

```text
Budget: 32,000 tokens

System:              4,000
Conversation:        8,000
Task:                1,000
Relevant code:      12,000
Tool output:         3,000
Memory:               2,000
Reserved:             2,000
----------------------------
Total:               32,000
```

The budgeter should prevent optimizers from exceeding the target.

---

## 19. Reversible Context

Every optimization operation should optionally create a reference:

```text
ctxc://context/<id>
```

Example:

Original:

```text
[10,000 tokens]

        |
        v
```

Optimized:

```text
[2,000 tokens]
Reference: ctxc://context/abc123
```

Retrieval:

```bash
ctxc retrieve ctxc://context/abc123
```

This enables safe aggressive optimization.

---

## 20. Tool Output Optimization

Tool output is one of the primary CtxC use cases.

Support:

- shell commands
- git
- npm
- pnpm
- yarn
- pytest
- cargo
- go
- docker
- curl
- jq
- test runners
- build systems

Do not hardcode every command into the core.

Use a handler registry:

```text
ToolOutputHandler
      |
      +-- GitHandler
      +-- TestHandler
      +-- BuildHandler
      +-- JsonHandler
      +-- GenericHandler
```

Generic fallback must always exist.

---

## 21. Shell Integration

Shell integration should be optional.

Supported environments:

- Windows
- PowerShell
- CMD
- macOS/Linux
- Bash
- Zsh
- Fish

Never assume Bash exists.

Shell-specific code must live outside the core.

---

## 22. Agent Integrations

CtxC should use an adapter architecture.

```rust
trait AgentIntegration {
    fn name(&self) -> &str;

    fn detect(&self) -> bool;

    fn install(&self) -> Result<()>;

    fn uninstall(&self) -> Result<()>;

    fn status(&self) -> Result<IntegrationStatus>;
}
```

Potential integrations:

- Claude Code
- Codex
- Gemini CLI
- Cursor
- GitHub Copilot
- Cline
- OpenCode
- Aider

Integrations must not be tightly coupled to the optimization engine.

---

## 23. MCP

CtxC should expose an MCP server.

Potential tools:

- ctxc_search
- ctxc_retrieve
- ctxc_compile
- ctxc_optimize
- ctxc_index
- ctxc_memory

Example:

```text
Agent
  |
  | MCP
  v
CtxC
  |
  +-- Search
  +-- Retrieval
  +-- Graph
  +-- Memory
  +-- Optimization
```

MCP should be an integration layer, not part of the core engine.

---

## 24. Local Daemon

The daemon is the CtxC runtime, not an optional accelerator.

```bash
ctxc start
```

Composition:

```text
ctxc
 |
 +-- CLI
 |
 +-- Daemon
       |
       +-- Project Manager
       +-- File Watcher
       +-- Indexer
       +-- Context Engine
       +-- Graph
       +-- Cache
       +-- Metrics
       +-- HTTP API
       +-- Dashboard
       +-- MCP
       +-- Agent Integrations
```

The daemon provides:

- Context cache
- Repository indexes
- Graph
- Memory
- Project registry
- Continuous file watching
- Incremental indexing
- Metrics collection
- HTTP API
- Dashboard hosting
- MCP
- Proxy

CLI commands should communicate with the daemon when it is available.

Example:

```text
ctxc optimize
      |
      v
   daemon
      |
      +-- existing index
      +-- existing graph
      +-- existing cache
```

Avoid rebuilding project state for every CLI invocation.

When the daemon is not running, one-shot commands must still work by constructing transient state. Reduced performance is acceptable; failure is not.

### 24.1 Status

The daemon must be inspectable at any time without attaching a debugger or reading a log. A single status query answers: is a daemon running, which projects it is looking after, how many are being watched versus scanned, how much has been indexed, and what has been saved cumulatively.

Status must always answer. A daemon that cannot be reached is reported as not running rather than as an error — the question "is anything running?" must never itself fail.

See [USAGE.md](USAGE.md#ctxc-status) for the command and its output.

### 24.2 Lifecycle

The daemon must:

- Bind to localhost only by default
- Write a lockfile/PID file to the platform data directory
- Refuse to start twice for the same data directory
- Shut down cleanly, flushing metrics and index state
- Recover from an unclean shutdown without corrupting the database
- Support optional auto-start on first CLI use

Auto-start must be configurable and must be disabled in CI environments.

---

## 25. Project Registry

CtxC manages a registry of projects rather than requiring a path on every invocation. The registry is what turns CtxC from a command you run into something that knows about your work.

Each registered project exposes:

- path
- identifier
- status
- watch status
- indexing status
- detected characteristics
- agent integrations
- token metrics
- optimization metrics
- cache statistics
- context graph statistics
- last activity

The registry is the shared source of truth for the CLI, the watcher, the metrics subsystem, and the dashboard.

Three properties matter architecturally:

- **Stable identity.** Project identity should survive a rename or a move, so an index, its metrics, and its memory are not orphaned by a relocated directory. Prefer a generated identifier stored in the project's own configuration over deriving identity from the path alone.
- **Status without loss.** Pausing a project takes it out of the watch set without discarding what CtxC already knows about it.
- **Detection as a hint.** Detected characteristics inform ignore defaults, parser selection, and ranking heuristics, but never gate behavior, and must always be re-runnable.

Registry operations write to the database directly rather than through the daemon, so they behave identically whether or not one is running; the daemon picks up changes on its next pass.

Removing a project from the registry must not delete the project's files. It removes CtxC's tracking and nothing else.

See [USAGE.md](USAGE.md#ctxc-project) for the commands.

---

## 26. Continuous Mode

CtxC should continuously observe registered projects.

```text
                 CtxC Daemon
                      |
        +-------------+-------------+
        |             |             |
    acme-web      acme-api     acme-docs
        |             |             |
      Watcher       Watcher       Watcher
        |             |             |
        v             v             v
     Changes       Changes       Changes
```

Watching is a property of a registered project, not a flag repeated on every command.

A transient `--watch` flag may exist for one-shot use, but it must not be the primary model.

### 26.1 File Watching

Use a native cross-platform filesystem watcher in Rust.

The watcher must detect:

- create
- modify
- delete
- rename

Then update only what changed:

```text
auth.ts modified
     |
     v
Watcher
     |
     v
Determine affected symbols
     |
     v
Update AST
     |
     v
Update graph
     |
     v
Invalidate relevant cache
     |
     v
Re-index
```

Do not re-index the entire repository when one file changes.

This is critical for large repositories.

### 26.2 Watch Scope

The watcher must respect ignore rules before doing any work:

- project ignore patterns
- `.gitignore`
- CtxC defaults (`node_modules`, `target`, `dist`, `build`, `.next`, `.venv`, and similar)

Watching a directory that contains a large build output directory must not degrade the system.

Platform limits (inotify watch limits, macOS FSEvents behavior, Windows directory handles) must be handled explicitly. When a watcher cannot be established, degrade to periodic scanning and report the degradation in `ctxc status` rather than failing silently.

---

## 27. Background Processing Pipeline

The watcher must not aggressively process every event immediately.

```text
Filesystem Event
       |
       v
Event Debouncer
       |
       v
Change Analyzer
       |
       +---- irrelevant change → ignore
       |
       +---- relevant change
                    |
                    v
              Incremental Index
                    |
                    v
              Graph Update
                    |
                    v
              Cache Update
```

For example, if an editor saves a file fifteen times in two seconds:

```text
save
save
save
save
save
...
```

CtxC should collapse that into approximately:

```text
"auth.ts changed"
        |
        v
process once
```

rather than processing every filesystem event.

### 27.1 Requirements

- Debounce events per file with a short quiet period
- Coalesce bursts affecting many files into a single batch
- Skip files whose content hash is unchanged
- Detect renames rather than treating them as delete plus create
- Process work on a bounded background queue
- Prioritize recently touched files
- Yield to interactive requests; a CLI or agent request must never wait behind bulk re-indexing
- Be idempotent, so a replayed or duplicated event causes no corruption

Background work must be low priority by design. CtxC running in the background must never make the user's machine feel slower.

---

## 28. Project Profiles and Detection

### 28.1 Project Configuration

A project may carry its own CtxC configuration in its root, so that a repository's settings travel with it rather than living in one developer's machine-wide file.

Two properties make this worth having:

- **Identity that survives a move.** A project that declares its own id keeps its history — index, metrics, memory — across a rename or a relocation. Without it, identity is derived from the path, and a moved directory becomes a different project.
- **Repository-scoped behavior.** Watching, indexing, budgets, and ignore rules are properties of the code, not of the person checking it out.

CtxC must never write this file. Registering a project is an operation on CtxC's own state; modifying the project being registered would be a surprising side effect.

The file participates in the layered configuration chain described in section 33, sitting above the global file and below the environment.

The file's location, accepted keys, and which of them this build acts on are documented in [USAGE.md](USAGE.md#project-configuration).

### 28.2 Project Detection

CtxC should automatically detect project characteristics when a project is added.

```text
acme-web/
├── package.json        → Node/Next.js
├── tsconfig.json       → TypeScript
├── prisma/
├── .git/
└── ...
```

Producing:

```text
Framework:        Next.js
Language:         TypeScript
Database:         PostgreSQL
Package Manager:  npm
Git:              yes
```

Detection informs ignore defaults, parser selection, tool output handlers, and ranking heuristics.

Detection results are a hint, never a hard requirement. Detection must be re-runnable, and the user must be able to override any detected value in project configuration.

---

## 29. Metrics Subsystem

Metrics are a dedicated subsystem, not a side effect of logging.

```text
                 Operations
                     |
                     v
              Metrics Collector
                     |
          +----------+----------+
          |                     |
          v                     v
       SQLite              Live Events
          |                     |
          v                     v
      Historical             Dashboard
       Metrics
```

Every optimization operation emits an event:

```json
{
  "project_id": "acme-web",
  "operation": "optimize",
  "source": "git_diff",
  "timestamp": "...",
  "input_tokens": 12400,
  "output_tokens": 3200,
  "tokens_saved": 9200,
  "reduction_ratio": 0.742,
  "duration_ms": 42
}
```

### 29.1 Attributed Savings

Do not report everything as a single "tokens saved" number.

Track where reduction actually came from:

```text
Raw Input
    ↓
Filtered
    ↓
Deduplicated
    ↓
Compressed
    ↓
Selected
    ↓
Final Context
```

Example:

```text
Input:                20,000
Filtering:             4,000 saved
Deduplication:         2,000 saved
Compression:           5,000 saved
Relevance selection:   3,000 saved
--------------------------------
Final:                 6,000
```

This lets the system explain *why* context was reduced, which is far more valuable than reporting:

```text
70% reduction
```

### 29.2 Collected Metrics

- Token reduction, per stage
- Compression ratio
- Optimization latency
- Cache hit rate
- Context retrieval frequency
- Index freshness and lag
- Watch event volume
- Errors and degradations

Cost estimates must be clearly labeled as estimates and must be derived from configurable, model-specific rates. Token counts are estimates unless produced by the exact target tokenizer, and cost figures inherit that uncertainty.

### 29.3 Storage and Retention

- Raw events are written to SQLite
- Events are rolled up into hourly and daily aggregates
- Raw events are retained for a configurable window, then pruned
- Aggregates are retained long-term
- Metrics writes must never block an optimization operation

Metrics are local. They are never transmitted anywhere. Telemetry remains disabled by default and is a separate concern from local metrics.

---

## 30. Dashboard

The dashboard is an optional local web UI backed by the daemon.

```text
                    CtxC
                     |
              +------+------+
              |             |
            CLI          Dashboard
              |             |
              +------+------+
                     |
                  Daemon
                     |
       +-------------+-------------+
       |             |             |
    Metrics       Projects       Context
       |             |             |
    SQLite        SQLite        SQLite
```

It is served by the daemon on the daemon's own port, so there is no second server to run and no second port to authorize. Opening it is a matter of building a URL that carries the access token and handing it to a browser; see [USAGE.md](USAGE.md#ctxc-dashboard).

### 30.1 Global Overview

```text
CtxC Overview

Projects                    7
Active Projects             3
Total Optimizations    12,482
Tokens Before           84.2M
Tokens After            31.7M
Tokens Saved            52.5M
Reduction               62.3%
Estimated Cost Saved   $84.21
```

### 30.2 Project View

```text
Project: acme-web

Path:
~/Projects/acme-web

Status:
● Watching

Context Operations:
2,431

Tokens:
Before:       18.4M
After:         6.7M
Saved:        11.7M

Reduction:
63.5%

Cache Hit Rate:
81%

Files Indexed:
4,821

Last Activity:
2 minutes ago
```

### 30.3 History

Charts for:

- Tokens before and after
- Token savings over time
- Number of optimizations
- Average compression ratio
- Cache hit rate
- Context retrieval frequency
- Optimization latency
- Errors

Example:

```text
Token Usage

20M |       ╭──╮
15M |   ╭───╯  ╰──╮
10M |───╯         ╰───
 5M |
    +-------------------
      Mon Tue Wed Thu Fri
```

### 30.4 Project Management

```text
Projects

● acme-web          ~/Projects/acme-web
● acme-api          ~/Projects/acme-api
○ acme-docs         ~/Projects/acme-docs
● acme-worker       ~/Projects/acme-worker
```

The dashboard should be able to add, pause, resume, and remove projects through the HTTP API, using the same registry the CLI uses.

### 30.5 Real-Time Updates

The dashboard must update without a manual refresh.

```text
File changed
     |
     v
Watcher
     |
     v
Optimization
     |
     v
Metrics updated
     |
     v
WebSocket event
     |
     v
Dashboard updates
```

Live activity stream:

```text
Activity

22:31:04  Optimized git diff          -72%
22:31:02  Indexed auth.ts
22:30:58  Updated dependency graph
22:30:41  Optimized pytest output     -84%
22:30:32  Retrieved context           3.2K tokens
```

This makes CtxC substantially easier to understand and debug.

### 30.6 Technology

Do not build the dashboard with a Rust UI framework.

```text
React Dashboard
       |
       | HTTP / WebSocket
       |
       v
CtxC Daemon
       |
       v
Rust Core
       |
       v
SQLite
```

Stack:

- React
- TypeScript
- Vite
- Tailwind
- shadcn/ui

Constraints:

- The dashboard is shipped inside the CtxC binary as embedded static assets
- Users must never need to separately install a web application
- The dashboard is a client of the HTTP API and must have no privileged access to the core
- The core must remain fully usable with the dashboard disabled or removed
- The dashboard must not become a dependency of the optimization engine

Keeping the dashboard behind the HTTP API prevents it from contaminating the core.

### 30.7 Dashboard Security

- Bind to localhost by default
- Require a local token for API access, generated by the daemon and passed by `ctxc dashboard`
- Never expose the dashboard on a public interface without explicit configuration
- Treat displayed context content as untrusted and escape it; context may contain arbitrary repository content

---

## 31. HTTP API

The daemon exposes a local HTTP API. It is the daemon's only remote surface, and everything that talks to a running CtxC — the dashboard, the CLI's liveness checks, third-party tooling — goes through it.

The surface is organised into five groups:

```text
System      health, status, shutdown
Projects    list, add, show, remove, pause, resume, reindex
Context     search, optimize
Metrics     summary, per project, timeseries, breakdown, activity
Real-time   event stream
```

The event stream should publish:

- watch events
- index progress
- optimization results
- metric updates
- project status changes
- errors and degradations

Constraints:

- The API is versioned under `/v1`. A breaking change means a new version, not a changed response.
- Bind locally by default. Do not expose the daemon publicly unless explicitly configured.
- Every route except the health check requires the access token the daemon generates at startup and publishes in its lockfile. Health is exempt because something has to be able to ask whether a daemon is there before it can read anything else.
- The dashboard is a normal client of this API and receives no special privileges.
- The HTTP layer is isolated from the core engine: the core must not depend on HTTP, and no route may contain logic that the CLI cannot reach by another path.

The exact routes this build serves, and how to authenticate against them, are documented in [USAGE.md](USAGE.md#http-api).

---

## 32. Proxy

Proxy functionality should be optional.

Architecture:

```text
AI Application
      |
      v
localhost:ctxc
      |
      +-- inspect
      +-- optimize
      +-- cache
      +-- record
      |
      v
LLM Provider
```

The proxy must never require CtxC to understand every provider.

Provider adapters should be separate.

---

## 33. Configuration

Configuration is layered, and every layer is a partial document: each key is optional, and layers merge key by key rather than wholesale. A file that sets one value must not reset the rest.

```text
Built-in defaults
        ↓
Global config
        ↓
Project config
        ↓
Environment variables
        ↓
CLI arguments
```

Higher layers override lower layers.

The format is TOML, and files reject unknown keys. A typo must become an actionable error naming the offending key, never a setting that silently does nothing.

Validation happens once, at load time, at the point where the message can still name the key that is wrong. Components downstream receive a configuration that is already known to be coherent.

The default location must be platform appropriate; Linux-specific paths must not be hardcoded. See [section 36](#36-cross-platform-abstraction) for how directories are resolved.

Project configuration (see section 28) participates in this chain. A project may carry its own identity and settings so that they travel with the repository.

Note that `[telemetry]` refers to external transmission and remains disabled by default. `[metrics]` is purely local and is unrelated to it.

The full set of keys, their defaults, their environment-variable equivalents, and the file locations are documented in [USAGE.md](USAGE.md#configuration).

---

## 34. Privacy

CtxC is local-first.

By default:

- Do not upload source code
- Do not upload prompts
- Do not upload logs
- Do not upload context
- Do not send telemetry

Any external AI integration must be explicitly enabled.

The architecture should make data flow obvious.

Metrics, the project registry, and the dashboard are entirely local. Metrics are stored in the local SQLite database and are never transmitted. The dashboard reads them over localhost only.

A long-running background daemon that watches source code raises the stakes here. The daemon must not send anything off the machine, and its listening surface must remain local unless the user explicitly configures otherwise.

---

## 35. Performance Requirements

CtxC should optimize for:

### Startup

CLI commands should start quickly.

### Memory

Avoid loading entire repositories into memory unnecessarily.

### Concurrency

Parallelize independent operations.

Examples:

```text
file A ─┐
file B ─┼──> parser
file C ─┤
file D ─┘
```

### Incremental indexing

Do not re-index unchanged files.

Use file metadata:

- path
- mtime
- size
- hash

Only reprocess changed content.

### Idle cost

The daemon runs continuously, so its idle cost is a user-visible property.

Targets:

- Negligible CPU usage when no files are changing
- Bounded memory that does not grow with uptime
- Background indexing that yields to interactive requests
- No measurable impact on editor or build performance

A background service that users notice is a background service users disable.

---

## 36. Cross-Platform Abstraction

Platform-specific functionality must use an abstraction.

Examples:

```text
Platform
 ├── config_directory()
 ├── cache_directory()
 ├── data_directory()
 ├── executable_directory()
 ├── shell()
 ├── process()
 └── environment()
```

Implement:

- WindowsPlatform
- UnixPlatform

where appropriate.

Never scatter:

```rust
#[cfg(target_os = "...")]
```

through business logic.

Keep conditional compilation near platform boundaries.

---

## 37. Error Handling

Use structured Rust errors.

Recommended:

- thiserror
- anyhow

Library crates should use typed errors.

Application-level code may use anyhow.

Errors should provide actionable messages.

Bad:

```text
Error: failed
```

Good:

```text
Failed to open context database:
<path>

Reason:
database is locked

Try:
ctxc daemon status
```

---

## 38. Logging

Use structured logging.

Recommended:

- tracing
- tracing-subscriber

Verbosity is selectable on the command line and overridable from the environment, so a filter can be set without changing what a script passes.

Diagnostic output goes to stderr, always. stdout belongs to command results, which may be machine readable, and a single stray log line on it corrupts a consumer's parse.

Raising verbosity must show CtxC's own reasoning rather than a wall of dependency output: third-party crates stay quiet unless asked for by name.

---

## 39. Output Formats

Every command should render in four formats: `human`, `json`, `jsonl`, and `quiet`.

Both renderings must come from one value. A command produces a single serializable result that knows how to render itself for a person; human and machine output therefore cannot drift apart as the command changes.

Commands whose product is content rather than a report — optimization and compilation — split their streams: the content goes to stdout so it can be piped, and the summary goes to stderr. This keeps a pipeline correct in every format.

This makes CtxC scriptable. See [USAGE.md](USAGE.md#output-formats) for how each format behaves.

---

## 40. Token Counting

Token counting should be provider/model-aware when possible.

Do not assume all LLMs use the same tokenizer.

Create:

```text
Tokenizer
    |
    +-- OpenAI-compatible
    +-- Anthropic
    +-- Gemini
    +-- Generic estimator
```

A generic estimator should be available when exact tokenization is unavailable.

Token counting must be treated as an estimate unless using the exact target tokenizer.

---

## 41. Optimization Result

Optimization should return structured metadata.

Example:

```json
{
  "original_tokens": 12000,
  "optimized_tokens": 4200,
  "reduction_ratio": 0.65,
  "compression_ratio": 2.85,
  "preserved_fragments": 42,
  "removed_fragments": 18,
  "retrievable_fragments": 12,
  "savings_by_stage": {
    "filtering": 4000,
    "deduplication": 2000,
    "compression": 5000,
    "selection": 3000
  }
}
```

This enables benchmarking and observability.

---

## 42. Quality Metrics

Do not measure success only by token reduction.

Track:

- Token Reduction
- Compression Ratio
- Latency
- Memory Usage
- Retrieval Accuracy
- Information Preservation
- Task Success
- Cache Hit Rate

- Index Freshness

A future benchmark should evaluate:

> Token Reduction vs Task Accuracy

The goal is to find the optimal point rather than maximize compression blindly.

These are the quality targets. Section 29 defines the subsystem that records them.

---

## 43. Testing Strategy

Use several levels of tests.

### Unit Tests

Test:

- Parsers
- Optimizers
- Routers
- Ranking
- Token budgets
- Serialization

### Integration Tests

Test:

```text
CLI → Engine → Optimizer → Store
```

### Cross-platform Tests

CI must test:

- Ubuntu
- Windows
- macOS

### Golden Tests

Maintain input/output fixtures:

```text
fixtures/
    git/
    json/
    logs/
    code/
    markdown/
```

Example:

```text
input.txt
expected.json
```

Optimization changes should be detected automatically.

### Daemon and Watcher Tests

Test:

- Daemon start, stop, and restart
- Registry persistence across restarts
- Watch event debouncing under save bursts
- Rename detection
- Ignore rule enforcement
- Incremental re-index correctness versus a full re-index
- Behavior when the watcher cannot be established

Filesystem watching behaves differently on each platform. These tests must run on all three.

### Benchmark Tests

Measure:

- Runtime
- Memory
- Token reduction
- Idle daemon CPU and memory
- Time from file change to updated index

---

## 44. Security

Treat all external input as untrusted.

Especially:

- Shell output
- Files
- API responses
- MCP input
- Proxy requests
- Agent messages

CtxC must never execute commands simply because they appear inside context.

Command execution must be explicit.

The optimizer must be incapable of turning arbitrary context into implicit code execution.

---

## 45. Dependency Policy

Prefer mature, focused dependencies.

Avoid adding dependencies for trivial functionality.

Every dependency should have a reason.

Core CtxC should remain lightweight.

Do not introduce:

- Python runtime
- Node runtime
- Database server
- Container runtime

as required dependencies.

---

## 46. Optional AI/ML Layer

AI-based optimization should live behind interfaces.

Example:

```rust
trait SemanticOptimizer {
    fn analyze(&self, context: &Context) -> Result<SemanticAnalysis>;

    fn optimize(
        &self,
        context: &Context,
    ) -> Result<OptimizedContext>;
}
```

Possible implementations:

- NoopSemanticOptimizer
- LocalEmbeddingOptimizer
- ONNXOptimizer
- ExternalLLMOptimizer

The core engine must work with:

- NoopSemanticOptimizer

This preserves local deterministic operation.

---

## 47. Implementation Phases

Do not implement the entire architecture at once.

### Phase 1 — Foundation

Implement:

- Rust workspace
- CLI
- Core types
- Configuration
- Logging
- Cross-platform paths
- Error handling
- Basic SQLite storage

Commands:

```bash
ctxc version
ctxc status
ctxc config
```

### Phase 2 — Context Engine

Implement:

- Context representation
- Context ingestion
- Content detection
- Token estimation
- Context fragments
- Basic optimization pipeline

Commands:

```bash
ctxc analyze <input>
ctxc optimize <input>
ctxc compile <input>
```

### Phase 3 — Tool Output

Implement:

- stdin processing
- Shell command processing
- Git handlers
- JSON optimization
- Log optimization
- Generic fallback

Example:

```bash
git status | ctxc optimize
```

### Phase 4 — Code Intelligence

Implement:

- Tree-sitter
- Symbol extraction
- Imports
- References
- Dependency graph
- Incremental indexing

Commands:

```bash
ctxc index .
ctxc graph .
ctxc search "authentication"
```

### Phase 5 — Retrieval

Implement:

- FTS5
- Context ranking
- Graph expansion
- Relevant context selection
- Context references

Commands:

```bash
ctxc search "authentication timeout"
ctxc retrieve ctxc://context/abc123
```

### Phase 6 — Daemon and Project Registry

Implement:

```bash
ctxc start
ctxc stop
ctxc status
ctxc daemon status

ctxc project add <path>
ctxc project list
ctxc project remove <path>
ctxc project pause <path>
ctxc project resume <path>
```

Add:

- Daemon lifecycle and lockfile
- Project registry
- Project detection
- Project configuration
- HTTP API
- Persistent cache
- Background indexing

### Phase 7 — Continuous Mode

Implement:

- Cross-platform file watcher
- Ignore rule handling
- Event debouncing
- Change analysis
- Incremental re-indexing on change
- Graph and cache invalidation
- Degradation to periodic scanning when watching is unavailable

At the end of this phase, this should be the whole setup:

```bash
ctxc project add ~/Projects/my-project
ctxc start
```

### Phase 8 — Metrics

Implement:

- Metrics collector
- Per-operation events
- Stage-attributed savings
- SQLite storage and rollups
- Retention and pruning
- Metrics HTTP endpoints

Commands:

```bash
ctxc metrics
ctxc metrics --project my-project
```

### Phase 9 — Dashboard

Implement:

- React/TypeScript dashboard
- Asset embedding into the binary
- Global overview
- Project views
- History charts
- WebSocket event stream
- Live activity feed
- Project management from the UI

Commands:

```bash
ctxc dashboard
```

### Phase 10 — Agent Integrations

Implement adapters incrementally.

Start with agents that provide the most useful integration mechanisms.

Each integration must be independently installable.

### Phase 11 — MCP

Implement:

- MCP server
- Search tool
- Retrieve tool
- Compile tool
- Optimize tool
- Memory tool

### Phase 12 — Semantic Intelligence

Only after deterministic infrastructure is stable:

- Embeddings
- Semantic retrieval
- Semantic compression
- Optional local models
- Optional external LLM providers

---

## 48. Initial MVP

The first usable CtxC release should NOT attempt to implement:

- Knowledge graphs
- Embeddings
- Local LLMs
- AI compression
- Every agent integration
- Proxy
- MCP

The MVP should focus on:

```text
Rust core
    +
Cross-platform CLI
    +
Context representation
    +
Token estimation
    +
Tool output optimization
    +
JSON/log/code-aware optimization
    +
SQLite storage
    +
Project registry
    +
File watcher
    +
Incremental indexing
    +
Metrics collection
    +
Local daemon
    +
Basic local dashboard
```

The MVP should already provide measurable token savings.

The daemon, project registry, and watcher are part of the MVP because they define what CtxC *is*. A version that only optimizes when explicitly invoked is a different, weaker product.

The dashboard in the MVP can be minimal: overview, project list, and live activity. Historical charts can follow.

---

## 49. Definition of Done for MVP

The MVP is complete when the following works on:

- macOS
- Linux
- Windows

with no runtime dependency.

### One-shot

```bash
ctxc optimize < input.txt
```

and:

```bash
ctxc capture -- cargo test
```

produce:

```text
Original tokens:     18,420
Optimized tokens:     7,230
Reduction:             60.7%
```

while preserving structured access to the original content.

### Managed

```bash
ctxc project add ~/Projects/my-project
ctxc start
```

and from that point onward CtxC works in the background, keeping the project's index, graph, and cache current as files change.

The user can inspect it at any time:

```bash
ctxc status
```

or:

```bash
ctxc dashboard
```

Specifically, the managed path is done when:

- A registered project is watched and incrementally re-indexed on change
- Editor save bursts do not cause repeated full re-indexing
- The daemon survives restart with its registry, index, and metrics intact
- `ctxc status` reports accurate cumulative savings
- The dashboard opens locally and updates live without a refresh
- Idle CPU usage is negligible

---

## 50. Future Architecture

The long-term architecture should evolve toward:

```text
                    AI Ecosystem
                         |
             +-----------+-----------+
             |           |           |
           Agents      IDEs       Apps
             |           |           |
             +-----------+-----------+
                         |
                    CtxC Runtime
                         |
       +-----------------+-----------------+
       |                 |                 |
   Context OS       Intelligence       Integrations
       |                 |                 |
   Acquisition       AST/Graph         MCP
   Storage           Retrieval         Proxy
   Memory            Ranking           Hooks
   Cache             Semantic          SDK
       |                 |                 |
       +-----------------+-----------------+
                         |
                  Optimized Context
                         |
                         v
                        LLM
```

CtxC should eventually become a general-purpose context runtime for AI agents, not merely a token reduction utility.

---

## 51. Architectural North Star

Every major feature should answer this question:

> Does this help an AI agent receive the minimum amount of context necessary to perform the task correctly?

If yes, it belongs in the CtxC ecosystem.

If it only reduces token count while degrading information quality, it does not.

The fundamental pipeline is:

```text
             RAW INFORMATION
                    |
                    v
              UNDERSTAND
                    |
                    v
                RELATE
                    |
                    v
                RETRIEVE
                    |
                    v
                 RANK
                    |
                    v
               TRANSFORM
                    |
                    v
               COMPRESS
                    |
                    v
                BUDGET
                    |
                    v
             OPTIMIZED CONTEXT
                    |
                    v
                  AGENT
                    |
                    v
             RETRIEVE ON DEMAND
```

CtxC should optimize context, not simply delete it.
