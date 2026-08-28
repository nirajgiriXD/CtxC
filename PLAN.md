# Plan: make CtxC easier to use, and faster

Two goals, kept separate because they need different work:

- **Easier** — shorten the distance between "I heard about CtxC" and "CtxC is
  saving me tokens".
- **Faster** — cut the wall-clock cost of indexing and searching, and cut the
  tokens CtxC itself spends.

Each item says what is true today, what to change, and how to tell it worked.

---

## Part 1 — Easier to use

### 1.1 Ship binaries (highest impact)

**Today.** [USAGE.md](USAGE.md) says: install Rust 1.77, install Node 22,
`npm ci`, `npm run build`, `cargo build --release`, then copy the binary onto
`PATH`. A cold release build of this workspace takes minutes. Anyone without a
Rust toolchain stops here.

**Change.** Add `.github/workflows/release.yml`, triggered on a `v*` tag:

- Build the dashboard once, then build `ctxc` for
  `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  and `x86_64-pc-windows-msvc`.
- Attach the archives and a `SHA256SUMS` file to the GitHub release.
- Add `install.sh` and `install.ps1` that detect the platform, download,
  verify the checksum, and place the binary.

**Done when.** A machine with no Rust and no Node gets a working `ctxc` from
one install command.

**Note.** [crates/ctxc-cli/src/commands/update.rs](crates/ctxc-cli/src/commands/update.rs)
rebuilds from a source checkout. Once releases exist it should prefer the
release asset and fall back to the source build. That is a change inside
`update.rs`, not a new command.

---

### 1.2 One command to get started: `ctxc init`

**Today.** First run is four commands (`status`, `project add`,
`project index`, `config agents install`), and the agent integrations are
found by reading [USAGE.md](USAGE.md).

**Change.** Add `ctxc init [PATH]` to
[crates/ctxc-cli/src/cli.rs](crates/ctxc-cli/src/cli.rs). It does the
following in order, printing each step:

1. Register the project (`project add`).
2. Index it (`project index`), with a progress line.
3. Detect the agents present and install CtxC guidance for those
   (`config agents install --detected`).
4. Offer to start the daemon (`--start` does it without asking).
5. Print the two or three commands to try next.

Flags: `--no-agents`, `--no-index`, `--start`, `--yes`. It must be safe to run
twice — every step it calls is already idempotent.

**Done when.** `ctxc init` in a fresh repository ends with an indexed project,
agents told about CtxC, and printed next steps.

---

### 1.3 Shell completions

**Today.** None. `clap_complete` is not a dependency.

**Change.** Add `clap_complete` and a hidden `ctxc completions <SHELL>` that
prints a script for bash, zsh, fish, PowerShell, and elvish. `ctxc init`
prints the one line that installs it for the current shell.

**Done when.** `ctxc <TAB>` lists the nine advertised commands, and
`ctxc project <TAB>` lists its subcommands.

---

### 1.4 `ctxc doctor`

**Today.** `ctxc status` reports paths, schema, and daemon state. It does not
say whether anything is *wrong*.

**Change.** Add `ctxc doctor`. It checks the following and reports pass or
fail, with the fix printed next to each failure:

- Database reachable, schema current, not locked.
- Data directory writable, and enough free disk space.
- Daemon running and answering; stale PID files.
- Registered projects whose path no longer exists.
- Registered projects never indexed, or indexed long ago.
- Agent integration blocks that point at a binary not on `PATH`.
- Whether the dashboard is compiled into this build.

**Done when.** Deleting a registered project's directory makes `ctxc doctor`
name it and print the `ctxc project remove` line that fixes it.

---

### 1.5 Say what to do next

**Today.** Empty results and empty state print facts, not directions.

**Change.** Where a result is empty, print the command that fills it:

- `ctxc find` with no index — "This project is not indexed. Run:
  `ctxc project index .`"
- `ctxc find` with no hits — suggest `--similar` and a wider `--limit`.
- `ctxc status` with no projects — suggest `ctxc init`.
- `ctxc dashboard` on a build with no UI — say so, and give the build line.

Write these to stderr, and only when `--format` is `human`, so piping and JSON
output stay clean.

**Done when.** Every empty-state path in
[crates/ctxc-cli/src/commands/](crates/ctxc-cli/src/commands/) ends with a
runnable next command, and JSON output is byte-identical to today.

---

### 1.6 Progress on long operations

**Today.** `ctxc project index` on a large repository prints nothing until it
finishes. A user cannot tell it from a hang.

**Change.** When stderr is a terminal and the format is `human`, print one
rewriting progress line: files seen, files parsed, elapsed. Print nothing when
piped. This pairs with 2.1, which needs a counter anyway.

**Done when.** Indexing this workspace shows a moving line, and
`ctxc --format json project index .` still produces parseable JSON.

---

### 1.7 Split the documentation

**Today.** [USAGE.md](USAGE.md) is 1,888 lines and
[ARCHITECTURE.md](ARCHITECTURE.md) is 2,609. Both are good; neither is
skimmable. No page answers "what do I type?" in ten seconds.

**Change.**

- Add a cheat sheet at the top of USAGE.md: one table, every command, one line
  each.
- Move troubleshooting into `TROUBLESHOOTING.md`, and link to it from the
  errors that send people there.
- Trim the README quick start to three commands: install, `ctxc init`,
  `ctxc find`.

**Done when.** A new user finds the command they need without scrolling
through 1,888 lines.

---

## Part 2 — Faster

Get a baseline before changing anything. Add a `benches/` directory with
criterion, or a committed script, that times these against a fixed corpus:

- cold index of this workspace,
- warm re-index with nothing changed,
- `ctxc find` with `--compile`,
- `ctxc optimize` over a 1 MB log.

Measure every item below against that baseline. No claim of "faster" without a
number.

---

### 2.1 Parallel indexing

**Today.** [crates/ctxc-engine/src/index.rs](crates/ctxc-engine/src/index.rs)
walks the project in a plain `for` loop: read, hash, parse, resolve imports,
write — one file at a time, on one core. Nothing in the workspace uses `rayon`
or `par_iter`.

**Change.** Split the loop into two phases:

1. **Parallel and pure.** Read, hash, parse, and resolve imports. No database.
   This is the expensive part, and it holds no shared state, so `par_iter`
   fits with little restructuring.
2. **Serial and writing.** Consume the results in order and write them inside
   the transaction that already wraps the pass
   ([commands/index.rs:149](crates/ctxc-cli/src/commands/index.rs#L149)).

SQLite stays a single writer. That is fine, because the writes were never the
slow part.

**Expected.** A cold index roughly `min(cores, 6)` times faster. This is the
largest single win available.

---

### 2.2 Cache prepared statements

**Today.** [crates/ctxc-store/src/index_store.rs](crates/ctxc-store/src/index_store.rs)
calls `conn.prepare(...)` and never `prepare_cached`. Every file indexed
re-parses and re-plans the same SQL.

**Change.** Replace `prepare` with `prepare_cached` through the store crate.
The change is mechanical, and rusqlite owns the cache.

**Expected.** 10 to 30 percent off the index write phase, for a very small
diff.

---

### 2.3 One fingerprint query, not N

**Today.** `index_one` starts with `self.store.file(root_key, relative)?` —
one `SELECT` per file, to answer "has this changed?". A 10,000-file repository
where nothing changed still runs 10,000 queries.

**Change.** Before the loop, load every stored fingerprint for the root with
one query into a `HashMap`. The unchanged check then costs a hash lookup. This
is what the module docstring already promises — "a `stat` per file, not a
parse per file" — taken one step further.

**Expected.** A warm re-index costs little more than the directory walk.

---

### 2.4 Make the vector scan cheaper

**Today.** [crates/ctxc-engine/src/embed.rs](crates/ctxc-engine/src/embed.rs)
loads *every* stored vector from SQLite on every similarity search, decodes
it, and compares. At 256 `f32` dimensions that is 1 KB per file, so 20,000
files is 20 MB read and decoded per query.

**Change,** in increasing order of effort:

1. Cache the decoded vectors in the daemon, keyed by root, invalidated on an
   index write. Repeat searches then touch no disk.
2. Store vectors as `i8` with a per-vector scale. That quarters the bytes, and
   cosine similarity over normalized vectors tolerates it. Needs a migration
   and a provider-version bump, so old rows are re-embedded rather than
   misread.
3. Only if 1 and 2 are not enough: an approximate index. The docstring's claim
   that brute force is cheap enough is probably still true at the sizes CtxC
   targets. Measure before building this.

**Expected.** Step 1 removes the cost for repeat queries, which are the common
case. Do step 1, measure, and stop there if it is enough.

---

### 2.5 Bound the deduplication scan

**Today.** `collapse_redundant` in
[crates/ctxc-semantic/src/similarity.rs](crates/ctxc-semantic/src/similarity.rs)
compares each fragment against every fragment kept so far. That is quadratic
in the worst case — where nothing is a duplicate, which is the normal case for
source code. A 5,000-fragment document is 12.5 million vector comparisons.

**Change.** Bucket by a cheap signature first — a few sign bits of the vector,
a random-hyperplane sketch — and compare only inside a bucket. Keep the exact
scan for small fragment counts, where the current code is already fast and
exactly right.

**Expected.** Large-input `optimize` goes from quadratic to near-linear.
Verify the verdicts are unchanged on the existing test corpus. This must not
change what CtxC decides, only how fast it decides it.

---

### 2.6 Cache optimizer results

**Today.** Optimizing the same input twice does all the work twice. The
content hash needed to notice is already computed during indexing.

**Change.** Key optimizer output by
`(content_hash, optimizer, budget, config_version)` in the existing database,
and return the stored result on a hit. Add `--no-cache` for anyone who needs
to force the work.

**Expected.** A repeated `ctxc optimize -- cargo test` becomes free while the
output does not change. This matters most for agents, which re-run the same
commands constantly.

---

### 2.7 Optional exact tokenizer

**Today.** The default tokenizer is an estimate, and
[crates/ctxc-core/src/token.rs](crates/ctxc-core/src/token.rs) is honest about
it. Budgets therefore carry a safety margin, so every optimized document
either wastes budget or risks overflowing it.

**Change.** Add an optional BPE tokenizer behind a Cargo feature, selectable
as `budget.tokenizer = "cl100k"`. Keep the estimator as the default, so the
no-dependency, no-download promise holds.

**Expected.** Tighter budgets, and `is_estimate()` finally returns `false` for
someone. This is a token-efficiency win rather than a speed win — but token
efficiency is the product.

---

## Order of work

Do these in order. Each one ships on its own.

| # | Item | Effort | Payoff |
|---|------|--------|--------|
| 1 | 2.2 `prepare_cached` | Hours | Free speed, tiny diff |
| 2 | 2.3 Batch fingerprints | Hours | Warm re-index |
| 3 | 1.2 `ctxc init` | 1 day | Onboarding |
| 4 | 1.5 Next-step hints | 1 day | Every user, every day |
| 5 | 2.1 Parallel indexing | 2 days | Biggest speed win |
| 6 | 1.1 Release binaries | 2 days | Removes the install wall |
| 7 | 1.3 Completions | Hours | Discoverability |
| 8 | 1.6 Progress output | 1 day | Pairs with item 5 |
| 9 | 1.4 `ctxc doctor` | 2 days | Support load |
| 10 | 2.4 Vector cache | 2 days | Search latency |
| 11 | 2.6 Optimizer cache | 2 days | Agent loops |
| 12 | 1.7 Docs split | 1 day | Skimmability |
| 13 | 2.5 Dedup buckets | 3 days | Large inputs |
| 14 | 2.7 Exact tokenizer | 3 days | Budget accuracy |

Items 1, 2, 5, 10, 11, and 13 need the baseline benchmark from Part 2 first.
Build that before item 1.

---

## What this plan does not change

- **The nine-command surface.** It was regrouped recently and it is right.
  `init`, `doctor`, and `completions` are additions, and `completions` stays
  hidden.
- **The hidden legacy command names.** They keep old scripts working and cost
  nothing.
- **The default embedder.** Feature hashing is deterministic, offline, and
  honest about being lexical. A trained model plugs in as an `Embedder` when
  someone wants one.
- **Local first.** Nothing here adds a network call to any operation except
  `ctxc update` and the install script.
