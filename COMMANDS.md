# CtxC commands

Two ways in. You type CLI commands in a terminal. Claude calls MCP tools
through `ctxc mcp`. Both run the same engine underneath.

## The 13 CLI commands (you type these)

| Command | Who | What it's for |
| --- | --- | --- |
| [`ctxc init`](USAGE.md#ctxc-init) | you | One-time setup per project. Registers, indexes, tells agents about it. |
| [`ctxc optimize`](USAGE.md#ctxc-optimize) | you + me | Shrink noisy text. You from a terminal, me via MCP. |
| [`ctxc find`](USAGE.md#ctxc-find) | you + me | Search your code, or get back the original behind a `ctxc://` link. |
| [`ctxc project`](USAGE.md#ctxc-project) | you | Add / list / re-index / remove projects. `graph` shows file deps. |
| [`ctxc start`](USAGE.md#ctxc-start) | you | Start the background daemon (auto re-indexes as you edit). |
| [`ctxc stop`](USAGE.md#ctxc-stop) | you | Kill the daemon and everything else. |
| [`ctxc doctor`](USAGE.md#ctxc-doctor) | you | "Why is it broken?" Prints the fix next to each problem. |
| [`ctxc status`](USAGE.md#ctxc-status) | you | Where files live, is the daemon up. `--metrics` = how much it saved. |
| [`ctxc config`](USAGE.md#ctxc-config) | you | See settings. `config agents install` wires up Claude/Cursor/etc. |
| [`ctxc dashboard`](USAGE.md#ctxc-dashboard) | you | Opens a web page with pretty graphs. |
| [`ctxc update`](USAGE.md#ctxc-update) | you | Upgrade CtxC itself. |
| [`ctxc mcp`](USAGE.md#ctxc-mcp) | nobody types this | Claude spawns it. It's the door I come in through. |
| [`ctxc completions`](USAGE.md#ctxc-completions) | you, once | Tab-completion for your shell. |

Each name links to its full flags, output, and examples in
[USAGE.md](USAGE.md#command-reference). Subcommands and mode flags have their
own sections there too, such as [`ctxc project index`](USAGE.md#ctxc-project-index),
[`ctxc optimize --dry-run`](USAGE.md#ctxc-optimize---dry-run), and
[`ctxc status --metrics`](USAGE.md#ctxc-status---metrics).

## The 6 MCP tools (Claude calls these)

| Tool | What it does |
| --- | --- |
| `ctxc_search` | Find the code that answers a question, without reading 40 files. |
| `ctxc_optimize` | Shrink a wall of build or test output before reading it. |
| `ctxc_compile` | Join several files into one document that fits a token budget. |
| `ctxc_retrieve` | Get back what `ctxc_optimize` removed. |
| `ctxc_index` | Re-read the project after large changes. |
| `ctxc_memory` | Keep notes across sessions. |

Their arguments are listed under [`ctxc mcp`](USAGE.md#ctxc-mcp), and they are
defined in [crates/ctxc-mcp/src/tools.rs](crates/ctxc-mcp/src/tools.rs).

## The split

- **You** do setup and health: `init`, `start`, `doctor`, `status`, `dashboard`, `update`.
- **Claude** does the daily work: search, shrink, retrieve.
- **Both** share `optimize` and `find`.

## Older names

Every command from an earlier version still runs. It is hidden from `--help`
rather than removed, so old scripts and aliases keep working. The mapping is
in [USAGE.md](USAGE.md#names-from-earlier-versions).
