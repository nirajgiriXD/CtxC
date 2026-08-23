# CtxC Usage Guide

Everything needed to install, configure, and operate CtxC.

This file is the single source of truth for user-facing instructions.
[README.md](README.md) explains what CtxC is and why it exists.
[ARCHITECTURE.md](ARCHITECTURE.md) explains how it works internally.

---

## Contents

- [Prerequisites](#prerequisites)
- [Installation](#installation)
- [Building from source](#building-from-source)
- [First run](#first-run)
- [Configuration](#configuration)
  - [Configuration files](#configuration-files)
  - [Configuration reference](#configuration-reference)
  - [Environment variables](#environment-variables)
  - [Project configuration](#project-configuration)
- [CLI overview](#cli-overview)
- [Command reference](#command-reference)
- [Common workflows](#common-workflows)
- [Watch mode](#watch-mode)
- [Storage and the database](#storage-and-the-database)
- [Output formats](#output-formats)
- [HTTP API](#http-api)
- [Logging and debugging](#logging-and-debugging)
- [Troubleshooting](#troubleshooting)
- [Updating](#updating)
- [Uninstalling](#uninstalling)
- [Platform notes](#platform-notes)

---

## Prerequisites

To **run** CtxC you need nothing but the binary. It is a single native
executable with no runtime dependency — no Node, no Python, no database
server, no Docker.

To **build** CtxC you need:

| Tool | Version | Required for |
|------|---------|--------------|
| Rust toolchain (`cargo`, `rustc`) | 1.77 or newer | The binary |
| Node.js and npm | 22 recommended | The dashboard web UI (optional) |

A C toolchain is not needed separately: SQLite is vendored and compiled by
`rusqlite`'s bundled feature.

Supported platforms: **Windows**, **macOS**, and **Linux** (and other
Unix-like systems following the XDG conventions).

---

## Installation

CtxC does not yet publish prebuilt binaries or packages. Build it from
source, then put the resulting executable on your `PATH`.

---

## Building from source

```bash
git clone https://github.com/nirajgirixd/ctxc
cd ctxc
cargo build --release
```

The binary lands at:

- `target/release/ctxc` (macOS, Linux)
- `target/release/ctxc.exe` (Windows)

### Including the dashboard

The dashboard is a Vite/React app compiled into the binary at build time.
Rust builds fine without it — `ctxc dashboard` then reports that this build
carries no dashboard, and everything else works unchanged.

To get a binary that includes the dashboard, build the web UI **before**
`cargo build`:

```bash
cd crates/ctxc-dashboard/ui
npm ci
npm run build
cd ../../..
cargo build --release
```

`npm run build` writes `crates/ctxc-dashboard/ui/dist`, which the
`ctxc-dashboard` build script embeds. Re-run `cargo build` after any UI
change.

### Installing the binary

```bash
# Cargo's own install location (~/.cargo/bin, already on PATH for most setups)
cargo install --path crates/ctxc-cli

# Or copy it somewhere yourself
cp target/release/ctxc /usr/local/bin/          # macOS, Linux
```

On Windows, copy `target\release\ctxc.exe` into a directory on `PATH`.

### Verifying the build

```bash
ctxc version
```

```text
ctxc 0.1.0
platform: windows (x86_64)
schema:   7
```

### Running the test suite

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

---

## First run

CtxC works with no setup at all — the database and directories are created
on first use.

```bash
# 1. See what this installation looks like
ctxc status

# 2. Optimize something immediately
git status | ctxc optimize --from "git status"

# 3. Register a project and index it
ctxc project add .
ctxc index .

# 4. Search it
ctxc search "how are tokens counted"
```

`ctxc status` reports paths, the database, and whether a daemon is running:

```text
CtxC 0.1.0

Platform:   windows (x86_64)
Config:     C:\Users\you\AppData\Roaming\ctxc\config.toml  (not created, using defaults)
Data:       C:\Users\you\AppData\Local\ctxc
Cache:      C:\Users\you\AppData\Local\ctxc\cache
Database:   C:\Users\you\AppData\Local\ctxc\ctxc.db  (schema 7, 229.3 KB)
Contexts:   0
Indexed:    0 project(s)
Daemon:     not running
```

---

## Configuration

Configuration is layered. Later layers override earlier ones, key by key:

```text
built-in defaults  ->  configuration file  ->  environment  ->  command line
```

A file that sets one value does not reset the rest. Unknown keys are
rejected with an error naming the file, so a typo never fails silently.

`ctxc config path` shows the layers that were consulted:

```bash
ctxc config path
```

```text
Configuration layers (lowest precedence first)

  defaults     -
  file         C:\Users\you\AppData\Roaming\ctxc\config.toml  (not present)
  environment  -
  command line -
```

### Configuration files

CtxC reads one global configuration file, in TOML. Its location is
platform-appropriate:

| Platform | Configuration file | Data directory | Cache directory |
|----------|-------------------|----------------|-----------------|
| Windows | `%APPDATA%\ctxc\config.toml` | `%LOCALAPPDATA%\ctxc` | `%LOCALAPPDATA%\ctxc\cache` |
| macOS | `~/Library/Application Support/ctxc/config.toml` | `~/Library/Application Support/ctxc` | `~/Library/Caches/ctxc` |
| Linux / Unix | `$XDG_CONFIG_HOME/ctxc/config.toml` (default `~/.config/ctxc/config.toml`) | `$XDG_DATA_HOME/ctxc` (default `~/.local/share/ctxc`) | `$XDG_CACHE_HOME/ctxc` (default `~/.cache/ctxc`) |

On Windows, if `LOCALAPPDATA` is not set the roaming directory is used for
data as well. On Unix, relative `XDG_*` values are ignored, as the spec
requires.

Create a file containing the built-in defaults:

```bash
ctxc config init
ctxc config init --force    # overwrite an existing file
```

Use a different file for one command:

```bash
ctxc --config ./ctxc-local.toml status
```

Relocate every CtxC directory at once — useful for portable installs,
sandboxes, and CI:

```bash
export CTXC_HOME=/srv/ctxc     # config, data, and cache all live here
```

With `CTXC_HOME` set, the configuration file is `$CTXC_HOME/config.toml`,
the database is `$CTXC_HOME/ctxc.db`, and the cache is `$CTXC_HOME/cache`.

### Configuration reference

The complete file, with defaults:

```toml
[core]
mode = "local"                       # only "local" is accepted

[optimization]
enabled = true                       # false makes optimization a no-op passthrough
target_reduction = 0.5               # reserved; validated (0.0-1.0) but not yet applied

[retrieval]
enabled = true                       # reserved; not yet applied

[ranking]
keyword = 1.0                        # weight of full-text relevance
semantic = 0.0                       # weight of embedding similarity
symbol = 1.5                         # weight of a matching symbol name
graph = 0.5                          # weight of how much the project depends on a file
recency = 0.3                        # weight of how recently a file changed
hop_decay = 0.4                      # score multiplier per dependency-graph hop (0.0-1.0)
expansion_depth = 1                  # how far to follow the graph; 0 disables expansion
recency_half_life_days = 30.0        # days after which a file counts as half as recent

[graph]
enabled = true                       # reserved; not yet applied

[storage]
path = "auto"                        # "auto" = platform data dir; anything else is a literal path

[budget]
default = 32000                      # token budget when a command does not supply one

[daemon]
enabled = true                       # false refuses to serve the dashboard
auto_start = true                    # reserved; not yet applied
bind = "127.0.0.1"
port = 7717                          # 0 asks the OS for a free port

[watch]
enabled = true                       # false makes the daemon index on demand only
debounce_ms = 300                    # quiet period before a changed file is acted on
poll_interval_ms = 30000             # scan interval for projects that cannot be watched

[semantic]
enabled = false                      # embeddings cost index time and database size
provider = "hashed"                  # the only provider in this build
dimensions = 256
redundancy_threshold = 0.92          # similarity at which two texts count as the same (0.0-1.0)
diversity = 0.25                     # relevance traded for coverage when selecting (0.0-1.0)

[metrics]
enabled = true                       # local only; never transmitted
raw_retention_days = 30              # 0 keeps raw events forever
hourly_retention_days = 90           # daily aggregates are never pruned
cost_model = "unspecified"           # the model cost estimates assume
cost_per_million_input_tokens = 0.0  # 0.0 means no cost is reported at all
cost_currency = "USD"

[dashboard]
enabled = true                       # false makes `ctxc dashboard` refuse
port = 7718                          # reserved; the dashboard is served on daemon.port

[telemetry]
enabled = false                      # external transmission; unrelated to [metrics]
```

Keys marked *reserved* are accepted and validated, but no code acts on them
in this release.

Values are validated when they are loaded, and an invalid one names itself:

- `core.mode` must be `local`
- `optimization.target_reduction`, `ranking.hop_decay`,
  `semantic.redundancy_threshold`, and `semantic.diversity` must be `0.0`–`1.0`
- `budget.default`, `semantic.dimensions`,
  `ranking.recency_half_life_days`, and `dashboard.port` must be greater than zero
- `ranking.*` weights must be zero or more
- `metrics.cost_per_million_input_tokens` must be zero or more
- `dashboard.port` must differ from `daemon.port` (unless `daemon.port = 0`)

### Environment variables

Every configuration key has an environment variable named
`CTXC_<SECTION>_<KEY>`, uppercased. Environment values override the file
and are overridden by command-line flags. Booleans accept `true`/`false`,
`1`/`0`, `yes`/`no`, `on`/`off`.

| Variable | Sets |
|----------|------|
| `CTXC_CORE_MODE` | `core.mode` |
| `CTXC_OPTIMIZATION_ENABLED` | `optimization.enabled` |
| `CTXC_OPTIMIZATION_TARGET_REDUCTION` | `optimization.target_reduction` |
| `CTXC_RETRIEVAL_ENABLED` | `retrieval.enabled` |
| `CTXC_RANKING_KEYWORD` | `ranking.keyword` |
| `CTXC_RANKING_SEMANTIC` | `ranking.semantic` |
| `CTXC_RANKING_SYMBOL` | `ranking.symbol` |
| `CTXC_RANKING_GRAPH` | `ranking.graph` |
| `CTXC_RANKING_RECENCY` | `ranking.recency` |
| `CTXC_RANKING_HOP_DECAY` | `ranking.hop_decay` |
| `CTXC_RANKING_EXPANSION_DEPTH` | `ranking.expansion_depth` |
| `CTXC_RANKING_RECENCY_HALF_LIFE_DAYS` | `ranking.recency_half_life_days` |
| `CTXC_GRAPH_ENABLED` | `graph.enabled` |
| `CTXC_STORAGE_PATH` | `storage.path` |
| `CTXC_BUDGET_DEFAULT` | `budget.default` |
| `CTXC_DAEMON_ENABLED` | `daemon.enabled` |
| `CTXC_DAEMON_AUTO_START` | `daemon.auto_start` |
| `CTXC_DAEMON_BIND` | `daemon.bind` |
| `CTXC_DAEMON_PORT` | `daemon.port` |
| `CTXC_WATCH_ENABLED` | `watch.enabled` |
| `CTXC_WATCH_DEBOUNCE_MS` | `watch.debounce_ms` |
| `CTXC_WATCH_POLL_INTERVAL_MS` | `watch.poll_interval_ms` |
| `CTXC_SEMANTIC_ENABLED` | `semantic.enabled` |
| `CTXC_SEMANTIC_PROVIDER` | `semantic.provider` |
| `CTXC_SEMANTIC_DIMENSIONS` | `semantic.dimensions` |
| `CTXC_SEMANTIC_REDUNDANCY_THRESHOLD` | `semantic.redundancy_threshold` |
| `CTXC_SEMANTIC_DIVERSITY` | `semantic.diversity` |
| `CTXC_METRICS_ENABLED` | `metrics.enabled` |
| `CTXC_METRICS_RAW_RETENTION_DAYS` | `metrics.raw_retention_days` |
| `CTXC_METRICS_HOURLY_RETENTION_DAYS` | `metrics.hourly_retention_days` |
| `CTXC_METRICS_COST_MODEL` | `metrics.cost_model` |
| `CTXC_METRICS_COST_PER_MILLION_INPUT_TOKENS` | `metrics.cost_per_million_input_tokens` |
| `CTXC_METRICS_COST_CURRENCY` | `metrics.cost_currency` |
| `CTXC_DASHBOARD_ENABLED` | `dashboard.enabled` |
| `CTXC_DASHBOARD_PORT` | `dashboard.port` |
| `CTXC_TELEMETRY_ENABLED` | `telemetry.enabled` |

Three variables are not configuration keys:

| Variable | Effect |
|----------|--------|
| `CTXC_HOME` | Relocates the config, data, and cache directories to one place. Wins over every platform convention. |
| `CTXC_LOG` | Log filter, in `tracing` syntax. Falls back to `RUST_LOG`. Overrides `--verbose` and `--debug`. |
| `CTXC_SOURCE` | The source checkout [`ctxc update`](#ctxc-update) builds from. Overridden by `--source`. |

Standard platform variables also participate in path resolution: `APPDATA`
and `LOCALAPPDATA` on Windows, `HOME` on macOS, and `HOME`,
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `XDG_CACHE_HOME` on Unix.

### Project configuration

A repository may carry `.ctxc.toml` (or `ctxc.toml`) in its root. CtxC
never writes this file — registering a project does not modify the project.
It is read when the project is added:

```toml
[project]
id = "acme-web-1"        # identity that survives a rename or move
name = "acme-web"        # what `ctxc project list` shows
```

The file also accepts `[watch] enabled`, `[index] enabled`,
`[budget] default`, and `[ignore] patterns`. These parse, but no code acts
on them in this release; only `[project] id` and `[project] name` change
behavior today.

Ignore rules for indexing come from a built-in list plus the project's
`.gitignore`. The built-in list covers `.git/`, `.hg/`, `.svn/`,
`node_modules/`, `target/`, `dist/`, `build/`, `out/`, `.next/`, `.nuxt/`,
`.svelte-kit/`, `.venv/`, `venv/`, `__pycache__/`, `.mypy_cache/`,
`.pytest_cache/`, `.ruff_cache/`, `.tox/`, `.gradle/`, `vendor/`,
`coverage/`, `.cache/`, `.idea/`, `.vscode/`, `*.min.js`, and `*.min.css`.

---

## CLI overview

```text
ctxc [OPTIONS] <COMMAND>
```

### Global options

These work on every command and may appear before or after the subcommand.

| Flag | Default | Meaning |
|------|---------|---------|
| `--config <PATH>` | platform default | Use this configuration file instead |
| `--format <FORMAT>` | `human` | `human`, `json`, `jsonl`, or `quiet` |
| `-v`, `--verbose` | off | Log informational messages to stderr |
| `--debug` | off | Log debug messages to stderr (wins over `--verbose`) |
| `-h`, `--help` | — | Help for the command |
| `-V`, `--version` | — | Print the version |

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | The command failed |
| `2` | Usage error (bad flags or arguments) |

### Streams

Results go to **stdout**; logs and diagnostics always go to **stderr**. For
the commands whose product is content — `optimize`, `compile`,
`search --compile`, and `retrieve` — stdout carries the content and the
summary goes to stderr in `human` format, so piping is always safe:

```bash
ctxc optimize build.log > optimized.txt        # only the content is redirected
```

---

## Command reference

| Command | Purpose |
|---------|---------|
| [`analyze`](#ctxc-analyze) | Describe input and what optimizing it would save |
| [`optimize`](#ctxc-optimize) | Optimize input and write the result to stdout |
| [`compile`](#ctxc-compile) | Optimize several inputs into one AI-ready document |
| [`capture`](#ctxc-capture) | Run a command and optimize what it prints |
| [`retrieve`](#ctxc-retrieve) | Recover the original behind a `ctxc://context/<id>` reference |
| [`index`](#ctxc-index) | Index a project's files, symbols, and relationships |
| [`graph`](#ctxc-graph) | Show how a project's files depend on each other |
| [`search`](#ctxc-search) | Find the context most relevant to a question |
| [`similar`](#ctxc-similar) | Find the files closest to a text, by embedding similarity |
| [`project`](#ctxc-project) | Manage the projects CtxC looks after |
| [`start` / `stop` / `daemon`](#ctxc-start--ctxc-stop--ctxc-daemon) | Daemon lifecycle and diagnostics |
| [`metrics`](#ctxc-metrics) | Show what CtxC has saved |
| [`dashboard`](#ctxc-dashboard) | Open the local dashboard in a browser |
| [`integrations`](#ctxc-integrations) | Tell coding agents about CtxC |
| [`mcp`](#ctxc-mcp) | Serve CtxC over the Model Context Protocol |
| [`status`](#ctxc-status) | Show the state of this installation |
| [`version`](#ctxc-version) | Print version and build information |
| [`update`](#ctxc-update) | Fetch the latest source, build it, replace this binary |
| [`config`](#ctxc-config) | Inspect and create configuration |

---

### `ctxc analyze`

Describe an input and report what optimizing it *would* save. Nothing is
written and nothing is stored.

```text
ctxc analyze [INPUT] [--from <COMMAND>]
```

| Argument / flag | Meaning |
|-----------------|---------|
| `INPUT` | File to analyze. Omit it, or pass `-`, to read standard input. |
| `--from <COMMAND>` | The command that produced this input, so a tool-specific optimizer can claim it. |

```bash
ctxc analyze README.md
cat build.log | ctxc analyze --from "cargo build"
```

```text
README.md

Type:        markdown
Size:        6,343 bytes
Lines:       397
Fragments:   132  (26 duplicated)
Tokens:      1,801  (estimated)

Optimizer:   text
Projected:   1,717 tokens  (4.7% smaller)

Savings by stage:
  deduplication   84 saved
```

`analyze` takes a file, not a directory. To work on a whole project, use
[`ctxc index`](#ctxc-index) and [`ctxc search`](#ctxc-search).

---

### `ctxc optimize`

Optimize one input and write the result to stdout.

```text
ctxc optimize [INPUT] [--from <COMMAND>] [--budget <TOKENS>] [--no-store]
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `INPUT` | stdin | File to optimize. `-` also means standard input. |
| `--from <COMMAND>` | — | Attribute the input to a command, so tool-aware optimization applies. |
| `--budget <TOKENS>` | `budget.default` | Token budget for the result. |
| `--no-store` | off | Do not keep the original in the context database. |

```bash
ctxc optimize build.log
ctxc optimize build.log --budget 2000
git status | ctxc optimize --from "git status"
ctxc optimize notes.txt --no-store
```

The optimized content goes to stdout; the summary goes to stderr:

```text
Original tokens:  71
Optimized tokens: 28
Reduction:        60.6%  (token counts are estimates)

  filtering       43 saved

Reference:        ctxc://context/8e3b5df9d44d2a8e7ebb6f3fdd870ec8
```

The original is stored by default, so the `ctxc://context/<id>` reference
stays resolvable through [`ctxc retrieve`](#ctxc-retrieve). `--no-store`
skips that, and the reference will not resolve.

Piped input carries no provenance. `--from` is how a tool-specific
optimizer gets selected for it; without it, generic text optimization
applies.

Input is capped at 16 MB per context. Larger material is a job for
[`ctxc index`](#ctxc-index).

---

### `ctxc compile`

Optimize several inputs into one AI-ready document.

```text
ctxc compile <INPUT>... [--budget <TOKENS>] [--no-store]
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `INPUT...` | required | Files to compile, in the order they should appear. `-` reads standard input. |
| `--budget <TOKENS>` | `budget.default` | Token budget for the whole document. |
| `--no-store` | off | Do not keep the originals in the context database. |

```bash
ctxc compile src/auth.rs src/session.rs README.md --budget 4000 > context.txt
```

Each section is introduced by a header carrying the source and its
reference:

```text
=== src/auth.rs (ctxc://context/12ca38d914d82b9c0c61f192cf859499) ===
```

The summary on stderr lists what each section cost:

```text
Original tokens:  2,110
Optimized tokens: 499
Reduction:        76.4%  (token counts are estimates)

Sections:
  README.md                   1,801 -> 464 tokens
  LICENSE                     309 -> 0 tokens
```

A section reduced to zero tokens did not fit the budget.

---

### `ctxc capture`

Run a command and optimize what it prints. The command is executed
directly, without a shell.

```text
ctxc capture [--budget <TOKENS>] [--no-store] -- <COMMAND>...
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `COMMAND...` | required | The command to run, after `--`. |
| `--budget <TOKENS>` | `budget.default` | Token budget for the result. |
| `--no-store` | off | Do not keep the original output. |

```bash
ctxc capture -- cargo test
ctxc capture --budget 1500 -- npm run build
ctxc capture -- git -c color.ui=false status
```

The `--` separator is required; everything after it belongs to the captured
command, including its own flags. The captured command's exit code is
reported in the summary and in JSON output — it does not become CtxC's exit
code.

CtxC recognizes families of commands and optimizes their output
accordingly: `git`, test runners (`cargo test`, `go test`,
`npm`/`pnpm`/`yarn`/`bun test`, `pytest`, `jest`, and similar), and build
commands (`cargo build|check|clippy|run`, `go build|vet`,
`npm`/`pnpm`/`yarn`/`bun install|ci|run|build`, `make`, and similar).
Anything else gets a generic command-output profile.

---

### `ctxc retrieve`

Recover the original content behind a `ctxc://context/<id>` reference.

```text
ctxc retrieve <REFERENCE>
```

```bash
ctxc retrieve ctxc://context/8e3b5df9d44d2a8e7ebb6f3fdd870ec8
ctxc retrieve 8e3b5df9d44d2a8e7ebb6f3fdd870ec8      # the bare id also works
```

The original content goes to stdout; its metadata goes to stderr:

```text
Reference:    ctxc://context/1b39864e6f5de1375fe194950491ec42
Source:       <stdin>
Type:         plain_text
Size:         24 bytes
```

A reference resolves as long as the original is in the database. It will
not resolve if the command that produced it ran with `--no-store`, or if
the database has been cleared.

---

### `ctxc index`

Index a project's code: files, symbols, and how they relate. Indexing is
incremental — unchanged files are skipped — so re-running it is cheap.

```text
ctxc index [PATH] [--force]
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `PATH` | current directory | Project directory to index. |
| `--force` | off | Re-parse every file, even ones that look unchanged. |

```bash
ctxc index
ctxc index ~/Projects/acme-web
ctxc index . --force
```

```text
.

Scanned:       180
Indexed:       180
Unchanged:     0
Symbols:       2,719
Relationships: 10,843
Ignored:       4
Duration:      825 ms
```

Symbols and relationships are extracted with Tree-sitter for **Rust,
JavaScript, TypeScript, TSX, Python, and Go**. Files in other languages are
still indexed for full-text search; they simply contribute no symbols.

With `semantic.enabled = true`, embeddings are computed in the same pass
and the report gains an `Embedded:` line.

A directory does not need to be a registered project to be indexed.

---

### `ctxc graph`

Show how a project's files depend on each other. Requires an index.

```text
ctxc graph [PATH] [--file <RELATIVE_PATH>] [--limit <COUNT>]
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `PATH` | current directory | Project directory. |
| `--file <RELATIVE_PATH>` | — | Show one file's dependencies and dependents instead of a summary. |
| `--limit <COUNT>` | `10` | How many files to list in the summary. |

```bash
ctxc graph
ctxc graph --limit 5
ctxc graph --file crates/ctxc-core/src/config.rs
```

```text
C:\Users\you\Projects\ctxc

Files:  180
Nodes:  130
Edges:  451

Most depended on:
  crates/ctxc-core/src/lib.rs                     52 dependents
  crates/ctxc-store/src/lib.rs                    24 dependents
  crates/ctxc-cli/src/output.rs                   17 dependents
```

With `--file`, the report also lists unresolved imports — imports that did
not resolve to a file inside this project, such as external crates and
packages.

---

### `ctxc search`

Find the context most relevant to a question. Full-text matches, symbol
matches, and what the dependency graph says those files lean on, ranked
together. Requires an index.

```text
ctxc search <QUERY> [--path <PATH>] [--limit <COUNT>]
             [--compile] [--budget <TOKENS>] [--no-store]
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `QUERY` | required | What to look for. Quote a phrase to keep it together. |
| `--path <PATH>` | current directory | Project directory to search. |
| `--limit <COUNT>` | `20` | Maximum number of results. |
| `--compile` | off | Emit the selected context as one optimized document instead of a list. |
| `--budget <TOKENS>` | `budget.default` | Token budget for `--compile`. |
| `--no-store` | off | With `--compile`, do not keep the cited originals. |

```bash
ctxc search "token budget"
ctxc search "auth timeout" --limit 5
ctxc search "session handling" --path ~/Projects/acme-web
ctxc search "how are tokens counted" --compile --budget 4000 > context.txt
```

Listing results:

```text
crates/ctxc-core/src/token.rs:149  (2.44)
  defines  tokens, Tokenizer, TokenBudget, HeuristicTokenizer
  |
  |     fn tokens(self, length: usize) -> u32 {

3 of 118 candidates shown
```

Each result carries its score, the symbols it defines, and — for a file
pulled in through the dependency graph rather than matched directly — a
`related` line naming the match it came from and how many hops away it is.

With `--compile`, the selected files are optimized into one document within
the budget. The document goes to stdout and the summary to stderr; the
cited originals are stored so their references resolve.

Ranking weights are configurable under `[ranking]`. Embedding similarity
contributes only when `ranking.semantic` is above zero, so turning
embeddings on never silently changes what search returns.

---

### `ctxc similar`

Find the files closest to a piece of text by embedding similarity, rather
than by keyword.

```text
ctxc similar <TEXT> [--path <PATH>] [--limit <COUNT>]
```

| Argument / flag | Default | Meaning |
|-----------------|---------|---------|
| `TEXT` | required | The text to compare against — a phrase, a symbol name, an error. |
| `--path <PATH>` | current directory | Project directory. |
| `--limit <COUNT>` | `10` | How many files to list. |

This command requires embeddings:

```bash
# once, in configuration or the environment
export CTXC_SEMANTIC_ENABLED=true

ctxc index .                       # builds the embeddings
ctxc similar "token budget accounting" --limit 3
```

```text
  crates/ctxc-core/src/token.rs                       24.7%
  crates/ctxc-store/migrations/0001_init.sql          18.5%

3 result(s), scored by the `hashed` embedder.
Note: similarity is lexical: it finds shared wording, not shared meaning.
```

The `hashed` embedder — the only one in this build — needs no model and no
network, and its similarity is **lexical**. An empty result means "nothing
that uses these words", not "nothing exists". Results below 15% similarity
are not shown.

---

### `ctxc project`

Manage the projects CtxC looks after. These commands go straight to the
database, so they behave identically whether or not a daemon is running;
the daemon picks up changes on its next pass.

```text
ctxc project add <PATH> [--index]
ctxc project list
ctxc project status <PROJECT>
ctxc project pause <PROJECT>
ctxc project resume <PROJECT>
ctxc project remove <PROJECT>
ctxc project open <PROJECT>
```

`<PROJECT>` may be a project **id**, **path**, or **name** — whichever is
in front of you.

| Subcommand | Effect |
|------------|--------|
| `add <PATH>` | Register a project. Adding the same directory twice is not an error. `--index` indexes it straight away instead of leaving it to the daemon. |
| `list` | List registered projects with status and index size. |
| `status <PROJECT>` | Show one project, re-running detection. |
| `pause <PROJECT>` | Stop looking after a project, without forgetting it. |
| `resume <PROJECT>` | Start looking after it again. |
| `remove <PROJECT>` | Forget a project. Its files are never touched. |
| `open <PROJECT>` | Print the project's path, for a shell to act on. |

```bash
ctxc project add ~/Projects/acme-web
ctxc project add . --index
ctxc project list
ctxc project status acme-web
ctxc project pause acme-web
cd "$(ctxc project open acme-web)"
```

```text
PROJECT                 STATUS    INDEX     PATH
ctxc                    active    180 files C:\Users\you\Projects\ctxc
```

`project add` detects languages, frameworks, package manager, and whether
the directory is a git repository:

```text
ctxc

Id:         ffbfa14db34a110f
Path:       C:\Users\you\Projects\ctxc
Status:     active
Indexed:    180 files, 2,719 symbols
Languages:  rust
Packages:   cargo
Git:        yes
Last index: never
```

`ctxc project open` prints a path rather than launching anything — what
"open" means is yours to decide.

---

### `ctxc start` / `ctxc stop` / `ctxc daemon`

The daemon keeps registered projects indexed as their files change, serves
the HTTP API, and serves the dashboard.

```text
ctxc start [--detach]
ctxc stop [--all]
ctxc daemon [status|start|stop]
```

| Command | Effect |
|---------|--------|
| `ctxc start` | Run the daemon in this terminal (foreground). |
| `ctxc start --detach` | Start it in the background and report its pid and port. |
| `ctxc stop` | Stop the daemon **and every other CtxC process**, or clear a lockfile left behind. |
| `ctxc stop --all` | Also stop CtxC processes belonging to other data directories. |
| `ctxc daemon status` | Report what the daemon is doing. Also the default when no subcommand is given. |
| `ctxc daemon start` | Same as `ctxc start --detach`. |
| `ctxc daemon stop` | Stop the daemon only, leaving everything else running. |

```bash
ctxc start --detach
ctxc daemon status
ctxc stop
```

```text
Daemon:     running
PID:        34976
Port:       7717
Uptime:     2m 14s
Projects:   1 (1 active)
Indexed:    180 files
Watching:   1 project(s)
```

Running in the foreground is the honest default and the right choice under
a service manager, in a container, or in a terminal you can watch. Use
`--detach` when you just want it running.

A running daemon writes `daemon.lock` in the data directory, recording its
pid, port, and an access token. Liveness is decided by asking the recorded
port for `/v1/health`, not by looking up the pid, so an unclean shutdown is
recoverable — `ctxc stop` clears the leftover lockfile.

Set `daemon.port = 0` to have the operating system pick a free port; the
port it got is recorded in the lockfile.

#### What `ctxc stop` stops

The daemon is not the only thing CtxC runs. An agent that has CtxC configured
spawns `ctxc mcp`, and may never clean it up. `ctxc stop` ends those too:

```bash
ctxc stop
```

```text
Daemon stopped (pid 34976)
Stopped 2 other CtxC process(es)
            pid 41208  ctxc mcp
            pid 41533  ctxc mcp
```

The daemon is asked to stop over its API, so it finishes what it is doing and
removes its own lockfile. The rest have no such channel and are terminated.

Every long-running CtxC process writes a small record of itself into
`processes/` in the data directory, and removes it on the way out. That is how
`ctxc stop` finds processes nothing else knows about. A record left behind by a
process that was killed outright is cleared rather than acted on: pids get
reused, so a pid is checked against the executable it should be running before
anything is sent to it.

This is scoped to one data directory. Two installations pointed at two
`CTXC_HOME`s are two independent systems, and stopping one does not reach into
the other. `ctxc stop --all` does reach across, for a process that left no
record at all:

```bash
ctxc stop --all
```

Use `ctxc daemon stop` when you want the daemon stopped and nothing else.

#### The dashboard cannot do this

The dashboard has a **Stop daemon & dashboard** button, and that is exactly what
it stops. It cannot end the rest, and deliberately so: a web page must not be
able to terminate processes on the machine serving it.

What it does instead is tell you what is left. The System page has an **Also
running** section listing every other CtxC process — what it is, its pid, and
when it started — and both that section and the stop confirmation point you at
`ctxc stop`. The `POST /v1/shutdown` response carries the same fact:

```json
{ "stopping": true, "pid": 34976, "still_running": 2, "stop_command": "ctxc stop" }
```

`GET /v1/processes` is the full list:

```json
{
  "processes": [
    { "pid": 34976, "command": "start", "started_at": 1755950000000, "is_daemon": true },
    { "pid": 41208, "command": "mcp", "started_at": 1755950120000, "is_daemon": false }
  ],
  "others": 1,
  "stop_command": "ctxc stop"
}
```

Both read the records in `processes/` and check each pid is still the executable
it claims, so a process killed outright never shows up as running.

---

### `ctxc metrics`

What CtxC has actually saved, and where the saving came from. Everything is
local; nothing is transmitted.

```text
ctxc metrics [--project <PROJECT>] [--days <DAYS>] [--breakdown]
             [--by <hour|day>] [--activity <COUNT>]
```

| Flag | Default | Meaning |
|------|---------|---------|
| `--project <PROJECT>` | all projects | Restrict to one project, by id, path, or name. |
| `--days <DAYS>` | `30` | How far back to look. |
| `--breakdown` | off | Break the saving down by operation. |
| `--by <BUCKET>` | — | Show one row per `hour` or per `day`. |
| `--activity <COUNT>` | `0` | List this many recent operations. |

```bash
ctxc metrics
ctxc metrics --days 7 --breakdown
ctxc metrics --by day --activity 10
ctxc metrics --project acme-web --format json
```

```text
CtxC metrics — last 30 days

Operations        4
Tokens in         1,872
Tokens out        1,745
Tokens saved      127
Reduction         6.8%  (token counts are estimates)
Latency           208 ms average, 825 ms slowest
Cache hits        0.0%
Cost saved        not estimated  (set metrics.cost_per_million_input_tokens)

Savings by stage:
  filtering       43
  deduplication   84
```

Cost is reported only once you give CtxC a rate to work from:

```toml
[metrics]
cost_model = "your-model-name"
cost_per_million_input_tokens = 3.0
cost_currency = "USD"
```

The command rolls up aggregates before reading, so a CLI-only installation
with no daemon still sees everything it has done.

---

### `ctxc dashboard`

Open the local dashboard in a browser.

```text
ctxc dashboard [--no-open]
```

| Flag | Effect |
|------|--------|
| `--no-open` | Print the URL instead of launching a browser. |

```bash
ctxc dashboard
ctxc dashboard --no-open
```

```text
Dashboard:  http://127.0.0.1:7717/?token=f1213218e2f61c9ac988b0bdbfe9258b
Size:       682.5 KB
```

The dashboard is served by the daemon, on the daemon's port. If no daemon
is running, this command starts one and says so. The URL carries the access
token because a browser cannot be given a header to send — keep it to
yourself. It never leaves loopback.

If the binary was built without the web UI, the command says so plainly
rather than offering a URL that cannot work. See
[Including the dashboard](#including-the-dashboard).

If no browser can be launched — headless machines, containers, locked-down
desktops — the URL is printed instead. That is not an error.

#### What is in it

The dashboard is the interactive way to run CtxC. It is a client of the HTTP
API above and has no privileged path around it, so everything it does is
something you can also do from a terminal or a script.

| Section | What it is for |
|---------|----------------|
| Overview | What CtxC has saved, and what the daemon is doing now. |
| Projects | Add, pause, resume, re-index and remove projects; open one to see its detection, index counts and dependency graph. |
| Activity | Every operation as it happens, filtered by kind and outcome, with the message behind each failure. |
| Performance | Savings over time, by stage and by operation. |
| Context | Search a project's index, read a selected file, and open a `ctxc://context/<id>` reference. |
| Commands | Every command this build accepts, read from the binary itself, with a link to the equivalent control where there is one. |
| Settings | The configuration file, edited a key at a time. |
| System | Daemon, storage, observation, the recent log, and the routes this build answers. |

Press `Ctrl`/`Cmd` + `K` anywhere for the command menu.

#### The same thing, from either side

| CLI | Dashboard |
|-----|-----------|
| `ctxc project add` | Projects → Add project |
| `ctxc project list` | Projects |
| `ctxc project status` | Projects → open one |
| `ctxc project pause` / `resume` | Projects → row menu |
| `ctxc project remove` | Projects → row menu |
| `ctxc index` | Projects → Re-index |
| `ctxc search` | Context → Search |
| `ctxc retrieve` | Context → Reference |
| `ctxc graph` | Context → Dependencies |
| `ctxc metrics` | Performance |
| `ctxc config show` / `path` / `init` | Settings |
| `ctxc status` | System |
| `ctxc daemon stop` | System → Stop daemon & dashboard |

Commands with no row here produce a stream, wrap a process, or replace the
binary — `optimize`, `compile`, `capture`, `mcp`, `integrations`, `update`.
Those belong in a terminal, and the Commands page says so rather than implying
a button exists.

---

### `ctxc integrations`

Tell coding agents about CtxC by writing a marked block into the file each
agent reads. Uninstalling takes exactly that block back out and leaves
everything else in place.

```text
ctxc integrations list [--path <PATH>]
ctxc integrations install [NAME] [--path <PATH>] [--detected]
ctxc integrations uninstall [NAME] [--path <PATH>] [--detected]
```

| Flag | Meaning |
|------|---------|
| `NAME` | The integration to act on. Omit it to act on several. |
| `--path <PATH>` | Project directory. Defaults to the current one. |
| `--detected` | Only agents that look like they are in use here. |

Available integrations:

| Name | Agent | File written |
|------|-------|--------------|
| `claude-code` | Claude Code | `CLAUDE.md` |
| `agents-md` | Codex, OpenCode, Aider, and other `AGENTS.md` readers | `AGENTS.md` |
| `copilot` | GitHub Copilot | `.github/copilot-instructions.md` |
| `gemini` | Gemini CLI | `GEMINI.md` |
| `cursor` | Cursor | `.cursor/rules/ctxc.mdc` |
| `cline` | Cline | `.clinerules/ctxc.md` |
| `claude-code-mcp` | Claude Code (MCP server) | `.mcp.json` |
| `cursor-mcp` | Cursor (MCP server) | `.cursor/mcp.json` |

```bash
ctxc integrations list
ctxc integrations install --detected
ctxc integrations install claude-code
ctxc integrations install claude-code-mcp
ctxc integrations uninstall cursor
```

```text
C:\Users\you\Projects\ctxc

  claude-code   not detected  CLAUDE.md
  agents-md     not detected  AGENTS.md
  copilot       available     .github/copilot-instructions.md

1 detected agent(s) have no CtxC guidance yet.
Add it with `ctxc integrations install --detected`.
```

Each integration reports one of: `installed`, `outdated` (an older CtxC
block is present), `available` (the agent is detected but has no guidance),
or `not detected`.

The `*-mcp` integrations register CtxC as an MCP server, which actually
hands the agent CtxC's tools. The instruction files only tell the agent the
CLI exists. Both are worth having.

A named integration installs whether or not it was detected — setting one
up before its first run is entirely reasonable.

The guidance CtxC writes tells the agent to search this project, which only
works once something has indexed it. Run `ctxc project add .` or
`ctxc index .` first; the command reminds you if you have not.

---

### `ctxc mcp`

Serve CtxC over the Model Context Protocol on stdin and stdout.

```text
ctxc mcp
```

Agents spawn this; people rarely run it directly. stdout is the JSON-RPC
protocol stream, so the command prints nothing human-readable and puts
every diagnostic on stderr. It blocks until the client hangs up.

Register it with an agent using
[`ctxc integrations install claude-code-mcp`](#ctxc-integrations) or
`cursor-mcp`, or configure it by hand:

```json
{
  "mcpServers": {
    "ctxc": {
      "command": "ctxc",
      "args": ["mcp"]
    }
  }
}
```

The tools offered:

| Tool | Purpose | Required arguments |
|------|---------|--------------------|
| `ctxc_search` | Find the code most relevant to a question; with `compile=true`, return its content inside a budget | `query` |
| `ctxc_optimize` | Shrink noisy text and return a `ctxc://context/<id>` reference | `content` |
| `ctxc_compile` | Combine several files into one AI-ready document within a budget | `paths` |
| `ctxc_retrieve` | Recover the original text behind a reference | `reference` |
| `ctxc_index` | Index a project so it can be searched | — |
| `ctxc_memory` | Keep short project notes across sessions (`save`, `get`, `list`, `forget`) | `action` |

Optional arguments mirror the CLI: `project`, `limit`, `compile`, `budget`,
`force`, `source`, `key`, and `value`. Relative paths and an unnamed
project resolve from the agent's working directory.

---

### `ctxc status`

Show the state of this installation: version, platform, paths, database,
and the daemon.

```text
ctxc status
```

```bash
ctxc status
ctxc status --format json
```

See [First run](#first-run) for sample output. This command opens (and, on
a fresh install, creates) the database.

---

### `ctxc version`

Print version and build information, including the database schema version
this build migrates to.

```bash
ctxc version
ctxc version --format json
```

```text
ctxc 0.1.0
platform: windows (x86_64)
schema:   7
```

---

### `ctxc update`

Update CtxC in place: fast-forward a source checkout to the latest `main`,
build the release binary, and put it where the running one is.

```text
ctxc update [--check] [--source <PATH>] [--branch <BRANCH>] [--force]
            [--no-dashboard]
```

| Flag | Effect |
|------|--------|
| `--check` | Report what an update would do. Nothing is built or replaced. |
| `--source <PATH>` | The checkout to build from. Remembered after the first time. |
| `--branch <BRANCH>` | The branch to follow. Defaults to `main`. |
| `--force` | Update even when the checkout is dirty, is on another branch, or has nothing new. |
| `--no-dashboard` | Skip rebuilding the dashboard's web interface. |

```bash
ctxc update --check
ctxc update
```

```text
Updated CtxC to 0.1.0.

Source:     /home/you/src/ctxc
Branch:     origin/main
Commit:     e880c3a -> 1d4e9ab  (12 commits)
Dashboard:  web UI rebuilt
Installed:  /usr/local/bin/ctxc
Daemon:     restarted
```

**Finding the source.** The checkout is looked for in this order: `--source`,
then `$CTXC_SOURCE`, then the path a previous update recorded, then the
directories above the running binary and above the current one. So a binary
still sitting in its own `target/release` needs no arguments at all, and
anything else needs `--source` exactly once:

```bash
ctxc update --source ~/src/ctxc
```

**What it refuses to do.** An update stops rather than guess when the checkout
has uncommitted changes, when it is on a branch other than the one being
followed, or when the running binary is a `target/debug` build. `--force`
overrides the first two; the third means you are working on CtxC, and should
run `cargo build --release` yourself.

**The dashboard.** An update keeps what you had. If the running binary carries
the dashboard, the web UI is rebuilt with `npm ci && npm run build` so the new
one carries it too — that needs npm, and the update says so if npm is missing.
If your binary has no dashboard, none is added.

**Replacing a running program.** The binary being replaced is the one running
the update, so it is renamed aside first — every platform allows that even
while the file is open. Windows cannot delete it until the process exits, so a
`ctxc.exe.ctxc-old` may sit next to the new binary until the next update
clears it. A daemon that was running is stopped just before the swap and
started again afterwards.

---

### `ctxc config`

Inspect and create configuration.

```text
ctxc config [show|path|init]
```

| Subcommand | Effect |
|------------|--------|
| `show` | Print the effective configuration after all layers are merged. The default when no subcommand is given. |
| `path` | Show which configuration layers were consulted, and where they live. |
| `init [--force]` | Write a configuration file containing the built-in defaults. `--force` overwrites an existing file. |

```bash
ctxc config
ctxc config show --format json
ctxc config path
ctxc config init
```

`config show` prints TOML, so what you see is exactly what a file would
contain. `config init` writes the **defaults**, not the effective
configuration — the file is a starting point to edit, and baking in
whatever environment variables happened to be set would surprise the next
run.

---

## Common workflows

### Shrink noisy command output before handing it to an agent

```bash
ctxc capture -- cargo test
ctxc capture -- npm run build
git diff | ctxc optimize --from "git diff"
```

### Assemble task-scoped context for an agent

```bash
ctxc index .
ctxc search "why does the session expire early" --compile --budget 6000 > context.txt
```

`context.txt` holds the highest-ranked files, optimized, inside the budget,
each cited by a reference the agent can expand with `ctxc retrieve`.

### Keep a project current in the background

```bash
ctxc project add ~/Projects/acme-web
ctxc start --detach
ctxc status
```

From that point on, CtxC keeps the project's index and graph current as
files change.

### Give a coding agent CtxC's tools

```bash
cd ~/Projects/acme-web
ctxc project add .
ctxc index .
ctxc integrations install --detected
ctxc integrations install claude-code-mcp
```

### Script CtxC from another tool

```bash
ctxc search "auth" --format json | jq '.files[].path'
ctxc metrics --format json | jq '.summary.tokens_saved'
ctxc optimize build.log --format quiet > optimized.txt
```

### Run a throwaway instance

```bash
CTXC_HOME=/tmp/ctxc-scratch ctxc index .
rm -rf /tmp/ctxc-scratch
```

---

## Watch mode

Watching is what the daemon does; there is no separate `watch` command.
Start the daemon and every **active** registered project is watched:

```bash
ctxc project add ~/Projects/acme-web
ctxc start --detach
ctxc daemon status
```

Behavior is controlled by `[watch]`:

| Key | Default | Meaning |
|-----|---------|---------|
| `enabled` | `true` | Turn watching off entirely. |
| `debounce_ms` | `300` | Quiet period before a changed file is acted on, so an editor's save burst does not cause repeated re-indexing. |
| `poll_interval_ms` | `30000` | How often a project that cannot be watched is scanned instead. |

Some projects cannot be watched — network filesystems, containers, and
platform watch limits. Those fall back to periodic scanning, and
`ctxc daemon status` reports them:

```text
Watching:   3 project(s)  (1 scanning instead)
            acme-web: <reason>
```

`ctxc project pause` takes a project out of the watch set without
forgetting it; `ctxc project resume` puts it back.

---

## Storage and the database

Everything CtxC persists lives in one SQLite database:

| Platform | Default database path |
|----------|-----------------------|
| Windows | `%LOCALAPPDATA%\ctxc\ctxc.db` |
| macOS | `~/Library/Application Support/ctxc/ctxc.db` |
| Linux / Unix | `~/.local/share/ctxc/ctxc.db` (or `$XDG_DATA_HOME/ctxc/ctxc.db`) |

It holds stored contexts (which is what makes `ctxc://context/<id>`
references resolvable), the file and symbol index, dependency
relationships, embeddings, the project registry, agent memory, and metrics.

Move it somewhere else:

```toml
[storage]
path = "/mnt/fast/ctxc.db"     # anything other than "auto" is used verbatim
```

Or, per run:

```bash
CTXC_STORAGE_PATH=/mnt/fast/ctxc.db ctxc status
```

The database is created and migrated automatically on first use.
`ctxc version` reports the schema version this build migrates to, and
`ctxc status` reports the schema version and size on disk of the database
you actually have.

Metrics retention is configurable: `metrics.raw_retention_days` (default
30; `0` keeps raw events forever) and `metrics.hourly_retention_days`
(default 90). Daily aggregates are the long-term record and are never
pruned.

Nothing else on your machine is touched. `ctxc project remove` forgets a
project; it never deletes files.

---

## Output formats

Every command accepts `--format`:

| Format | Behavior |
|--------|----------|
| `human` | Formatted for a person reading a terminal. The default. |
| `json` | One pretty-printed JSON document. |
| `jsonl` | One compact JSON document per line, for streaming consumers. |
| `quiet` | No report at all; the exit code carries the result. |

```bash
ctxc status --format json
ctxc search "auth" --format jsonl
ctxc optimize build.log --format quiet > optimized.txt
```

Human and machine output come from the same value, so they cannot drift
apart.

For the content-producing commands — `optimize`, `compile`,
`search --compile`, and `retrieve` — the formats differ in where the
content goes:

| Format | stdout | stderr |
|--------|--------|--------|
| `human` | the optimized content | the summary |
| `json` / `jsonl` | one document carrying both content and summary | — |
| `quiet` | the optimized content | — |

That keeps `ctxc optimize file | agent` correct in every format.

---

## HTTP API

The daemon serves a local HTTP API on `daemon.bind`:`daemon.port`
(`127.0.0.1:7717` by default), versioned under `/v1`.

Every route except `/v1/health` requires the daemon's access token as a
bearer token. The token is in `daemon.lock` in the data directory:

```bash
curl http://127.0.0.1:7717/v1/health

TOKEN=$(jq -r .token ~/.local/share/ctxc/daemon.lock)
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7717/v1/status
```

| Method | Route | Purpose |
|--------|-------|---------|
| `GET` | `/v1/health` | Liveness. The only unauthenticated route. |
| `GET` | `/v1/status` | Daemon status. |
| `GET` `POST` | `/v1/projects` | List or add projects. |
| `GET` `DELETE` | `/v1/projects/{id}` | Show or remove one project. |
| `POST` | `/v1/projects/{id}/pause` | Pause a project. |
| `POST` | `/v1/projects/{id}/resume` | Resume a project. |
| `POST` | `/v1/projects/{id}/reindex` | Re-index a project. |
| `POST` | `/v1/context/search` | Search a project. |
| `POST` | `/v1/context/optimize` | Optimize content. |
| `GET` | `/v1/metrics/summary` | Metrics summary. |
| `GET` | `/v1/metrics/projects/{id}` | Metrics for one project. |
| `GET` | `/v1/metrics/timeseries` | Metrics over time. |
| `GET` | `/v1/metrics/breakdown` | Metrics by operation. |
| `GET` | `/v1/activity` | Recent operations. |
| `GET` | `/v1/config` | Effective configuration, its layers, and where they live. |
| `PATCH` | `/v1/config` | Change settings in the configuration file. |
| `GET` | `/v1/commands` | The command tree this build accepts. |
| `GET` | `/v1/contexts/{id}` | Recover the original behind a `ctxc://context/<id>`. |
| `GET` | `/v1/projects/{id}/file` | One indexed file, by `?path=`. |
| `GET` | `/v1/projects/{id}/graph` | Dependency summary for a project. |
| `GET` | `/v1/diagnostics` | Version, paths, database and log capacity. |
| `GET` | `/v1/logs` | Recent daemon log records. |
| `GET` | `/v1/events` | Live event stream (WebSocket). |
| `POST` | `/v1/shutdown` | Stop the daemon. |

The WebSocket at `/v1/events` takes the token as a query parameter —
`?token=<token>` — because browsers cannot set headers on a handshake.

### Changing configuration over the API

`PATCH /v1/config` takes the keys to set and the keys to hand back to the
defaults. Only what it names is touched, comments in the file survive, and a
value that fails validation is refused before anything is written:

```bash
curl -X PATCH -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' \
     -d '{"set": {"watch": {"debounce_ms": 500}}, "reset": ["daemon.port"]}' \
     http://127.0.0.1:7717/v1/config
```

The response carries the keys that changed, any that a `CTXC_*` variable still
overrides, and `restart_required` — the daemon reads its configuration once, at
startup, so an edit does not reach a running one until it is restarted.

### Reading the daemon's log

The daemon keeps its recent records in memory, so a daemon started with
`--detach` can still be asked what it has been doing. Nothing is written to
disk; the oldest records fall out to make room, and the count that fell out is
reported:

```bash
curl -H "Authorization: Bearer $TOKEN" \
     'http://127.0.0.1:7717/v1/logs?level=warn&limit=50'
```

---

## Logging and debugging

Logs always go to **stderr**, so stdout stays safe to pipe.

```bash
ctxc index .            # warnings only (the default)
ctxc -v index .         # informational messages
ctxc --debug index .    # debug messages; --debug wins over --verbose
```

For finer control, set `CTXC_LOG` using `tracing` filter syntax. It falls
back to `RUST_LOG` and overrides the flags:

```bash
CTXC_LOG=ctxc_store=debug ctxc index .
CTXC_LOG=warn,ctxc_daemon=trace ctxc start
```

`--verbose` and `--debug` deliberately leave third-party crates at `warn`,
so `--debug` shows CtxC's own reasoning rather than a wall of dependency
output.

To see why a detached daemon will not start, run it in the foreground:

```bash
ctxc start
```

---

## Troubleshooting

**`<path> has not been indexed`**
Run `ctxc index` in that directory first. `search` and `graph` read the
index; they do not scan the filesystem.

**`no input given, and standard input is a terminal`**
`optimize` and `analyze` read stdin when no file is given. Pass a file, or
pipe something in: `git status | ctxc optimize`.

**`embeddings are switched off`**
`ctxc similar` needs embeddings. Set `semantic.enabled = true`, then
re-run `ctxc index` to build them.

**`<path> has no embeddings`**
Embeddings were turned on after the project was indexed. Run `ctxc index`
again.

**`nothing matching "..." fits a budget of N tokens`**
Raise `--budget`, or search for something narrower.

**`no context is stored for ctxc://context/<id>`**
The reference was produced with `--no-store`, or the database was cleared.
Re-run the command that produced it.

**`a daemon is already running (pid ..., port ...)`**
Check it with `ctxc daemon status`, or stop it with `ctxc stop`.

**`Daemon: not running (a lockfile was left behind)`**
A daemon exited uncleanly. `ctxc stop` clears the lockfile.

**`no daemon is running`** from `ctxc stop`
Nothing was running: no daemon, and no other CtxC process for this data
directory. Try `ctxc stop --all` if you believe one is running under a
different `CTXC_HOME`.

**`Could not stop pid <pid>: ...`**
The process refused to end, usually because it belongs to another user or is
being held open by a debugger. Run `ctxc stop` again, or end it yourself.

**`the daemon did not start`**
Run `ctxc start` in the foreground to see the failure.

**`the daemon is not answering on port <port>`**
Start it with `ctxc start`, or check `ctxc daemon status`.

**`the dashboard is disabled in configuration`**
Set `dashboard.enabled = true`, or use `ctxc metrics` instead.

**`This build of CtxC does not include the dashboard.`**
The binary was built without the web UI. See
[Including the dashboard](#including-the-dashboard).

**`failed to resolve CtxC directories for this platform`**
The variables CtxC needs are not set — `APPDATA` on Windows, `HOME`
elsewhere. Set `CTXC_HOME` to point every directory at one place.

**A configuration key is rejected**
Files reject unknown keys on purpose, so a typo is an error rather than a
setting that silently does nothing. Compare against
[the reference](#configuration-reference), or regenerate a known-good file
with `ctxc config init --force`.

**Input is too large**
A single context is capped at 16 MB. Larger material is a job for
`ctxc index`, not for one context.

---

## Updating

CtxC updates itself. See [`ctxc update`](#ctxc-update) for the full flag list.

```bash
ctxc update --check     # what would change
ctxc update             # pull main, build it, replace this binary
```

The first run needs to be told where the source is, unless the binary is
still in the checkout it was built in:

```bash
ctxc update --source ~/src/ctxc
```

The path is remembered, so later updates work from anywhere. A running daemon
is stopped for the swap and started again; the dashboard is rebuilt if the
binary had one.

To do it by hand instead:

```bash
cd ctxc
git pull
cd crates/ctxc-dashboard/ui && npm ci && npm run build && cd ../../..
cargo build --release
```

Stop everything CtxC is running before replacing the binary yourself — an MCP
server holding the old binary open will keep it from being overwritten:

```bash
ctxc stop
```

Either way, the database is migrated automatically the first time the new
binary opens it. Compare `ctxc version` (the schema this build wants) with
`ctxc status` (the schema you have) if you want to confirm a migration ran.

---

## Uninstalling

CtxC keeps everything in two places: the binary, and its own directories.
Removing them removes CtxC completely — no project files are ever touched.

```bash
# 1. Stop the daemon and everything else CtxC is running
ctxc stop

# 2. Take CtxC guidance back out of any project that has it
ctxc integrations uninstall --path ~/Projects/acme-web

# 3. Remove the binary
cargo uninstall ctxc-cli          # if installed with cargo install
rm /usr/local/bin/ctxc            # if copied by hand
```

Then delete the directories listed under
[Configuration files](#configuration-files) — or, if you set `CTXC_HOME`,
just that one directory:

```bash
# Linux
rm -rf ~/.config/ctxc ~/.local/share/ctxc ~/.cache/ctxc

# macOS
rm -rf ~/Library/Application\ Support/ctxc ~/Library/Caches/ctxc
```

```powershell
# Windows
Remove-Item -Recurse -Force "$env:APPDATA\ctxc", "$env:LOCALAPPDATA\ctxc"
```

If you moved the database with `storage.path`, delete that file too.

---

## Platform notes

### Windows

- Configuration lives under `%APPDATA%\ctxc`; data and cache under
  `%LOCALAPPDATA%\ctxc`. If `LOCALAPPDATA` is unset, the roaming directory
  is used for both.
- `ctxc start --detach` gives the daemon no console and its own process
  group, so it survives the terminal that started it and a Ctrl-C there
  does not reach it.
- `ctxc dashboard` launches a browser through `cmd /C start`.
- Paths are shown without the `\\?\` extended-length prefix.

### macOS

- Configuration and data share `~/Library/Application Support/ctxc`; the
  cache is `~/Library/Caches/ctxc`.
- `ctxc dashboard` launches a browser through `open`.

### Linux and other Unix

- XDG base directories are honored, and relative `XDG_*` values are ignored
  as the spec requires.
- `ctxc dashboard` launches a browser through `xdg-open`. On a headless
  machine it prints the URL instead, which is not an error.

### All platforms

- `CTXC_HOME` overrides every convention above and puts config, data, and
  cache in one directory.
- CtxC never assumes a particular shell. `ctxc capture` runs its command
  directly, without a shell, so shell syntax after `--` is not interpreted.
