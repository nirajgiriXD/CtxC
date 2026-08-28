# CtxC Troubleshooting

What each error means, and what to type about it. If nothing here matches,
`ctxc doctor` checks the installation itself and prints the fix next to
whatever it finds.

[USAGE.md](USAGE.md) is the full guide. [README.md](README.md) explains what
CtxC is.

---

## Start here

```bash
ctxc doctor
```

It checks the database, the data directory, the daemon, every registered
project, the agent integrations, and whether this build carries the dashboard.
Each failure is printed with the command that fixes it, and the exit code is
non-zero when something is wrong — so it works in a script as well as in a
terminal.

---

## Errors, and what to do about them

**`<path> has not been indexed`**
Run `ctxc project index` in that directory first. `find` and `project graph` read the
index; they do not scan the filesystem.

**`no input given, and standard input is a terminal`**
`optimize` reads stdin when no file is given. Pass a file, or
pipe something in: `git status | ctxc optimize`.

**`embeddings are switched off`**
`ctxc find --similar` needs embeddings. Set `semantic.enabled = true`, then
re-run `ctxc project index` to build them.

**`<path> has no embeddings`**
Embeddings were turned on after the project was indexed. Run `ctxc project index`
again.

**`nothing matching "..." fits a budget of N tokens`**
Raise `--budget`, or search for something narrower.

**`no context is stored for ctxc://context/<id>`**
The reference was produced with `--no-store`, or the database was cleared.
Re-run the command that produced it.

**`a daemon is already running (pid ..., port ...)`**
Check it with `ctxc status --daemon`, or stop it with `ctxc stop`.

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
Start it with `ctxc start`, or check `ctxc status --daemon`.

**`the dashboard is disabled in configuration`**
Set `dashboard.enabled = true`, or use `ctxc status --metrics` instead.

**`This build of CtxC does not include the dashboard.`**
The binary was built without the web UI. See
[Including the dashboard](USAGE.md#including-the-dashboard).

**`failed to resolve CtxC directories for this platform`**
The variables CtxC needs are not set — `APPDATA` on Windows, `HOME`
elsewhere. Set `CTXC_HOME` to point every directory at one place.

**A configuration key is rejected**
Files reject unknown keys on purpose, so a typo is an error rather than a
setting that silently does nothing. Compare against
[the reference](USAGE.md#configuration-reference), or regenerate a known-good file
with `ctxc config init --force`.

**Input is too large**
A single context is capped at 16 MB. Larger material is a job for
`ctxc project index`, not for one context.

---
