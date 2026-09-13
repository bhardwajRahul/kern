# kern-sandbox: the operational notes

The long tail, moved out of the package README so that page stays a landing page. Every item here was
measured on a real box and cost somebody an afternoon; none of it is needed to run your first call.

Read it when a box does something you did not expect: a build that fails with a network error that is
not one, a `df` that lies, output that vanishes past a cap, a chart that renders and complains anyway.

**Scratch does not survive a call, except in a `kernel()`.** Each `run_code` is a fresh box, so `/tmp`
is fresh too while the workspace persists. A `kernel()` is one long-lived box and the opposite holds:
its `/tmp` accumulates. Measured at 10 MiB per step under the 64 MiB default, ten `run_code` calls all
pass and ten kernel cells fail from the seventh with `OSError: [Errno 28]`. A read-only `/tmp` failed loudly at the moment of the mistake; now a tool that
writes state to the workspace and a lock to `/tmp` writes both, and the next call finds the state
pointing at a path that is gone. Put anything a later call must find in the workspace. The `setup=` box is the exception: an install needs unbounded
scratch, so the default is not applied there (an explicit `tmpfs=` still is).

**A fault ends a `kernel()`, and only the workspace comes back.** A cell that is killed (OOM, timeout,
a blocked syscall) takes the interpreter with it. Measured with `memory_mb=128`: cell A sets `x = 41` and
writes `keep.txt`, cell B allocates until the cap bites (`exit_code` 137, `fault.type == "oom"`), and from
there the two front ends differ on purpose. In the SDK the next `run_code` RAISES `kernel is dead: a prior
cell ended it (oom). Files written to the workspace are still there; names and imports from the earlier
cells are gone`, because a fresh interpreter handed back in silence would answer questions about state
that no longer exists. The MCP server cannot raise at a model, so it opens a fresh kernel and says so on
the reply for the cell that died; through it, cell C reads `x still there? False` with `keep.txt`
unchanged. Either way `fault` is the signal: if it is not `None`, the names are gone and the files are not,
so re-run the setup cell.

**Toolchains in the box.** npm, Go, Rust and .NET cache under `$HOME`, and `$HOME` is inside the
read-only root. The scratch at `/tmp` is half the answer; `HOME` is the other half, and no error says
so. Go reports `failed to initialize build cache at /root/.cache`, which is true and does not mention
`HOME`. npm is worse: a failed `mkdir /root/.npm` reaches the user as
`Invalid response body while trying to fetch https://registry.npmjs.org/express`, which reads as a
network fault and is not one. Measured on `node:22`: neither -> exit 2, `HOME` alone with a read-only
`/tmp` -> still exit 2, both -> exit 0.

```python
Sandbox(
    image="golang:1.23-alpine",
    env={"HOME": "/workspace"},   # npm's ~/.npm, Go's ~/.cache, Rust's CARGO_HOME, .NET's NuGet
    tmpfs={"/tmp": "512m"},       # scratch; 64 MiB fits a small install, a real one needs more
)
```

That message is verbatim from a box, and the recipe above is what makes the same build print its
output. **Point `HOME` at the workspace, not at the scratch**: `npm install webpack webpack-cli
typescript eslint` needs 81 MiB of cache, so `HOME=/tmp` fails with `ENOSPC` against the 64 MiB
default while `HOME=/workspace` succeeds. One small package fits either way, which is why testing
with `express` proves nothing.

**Two numbers inside a box describe the host, not your box, and a program will act on them.** `df`
reports a tmpfs's own size, and `nproc` reports the host's CPU count: measured under `cpus=0.5`,
`nproc` says 28 while `cpu.max` says `50000 100000`, so `make -j$(nproc)` starts 28 jobs against half
a core and a `pids` ceiling. The same shape reaches SQLite, which spills `CREATE INDEX` into `/tmp`:
a 309 MB database on the workspace fails with `database or disk is full` while `df /workspace` shows
202 GB free, and by the time you look, `/tmp` is empty again because SQLite cleaned up. Point
`TMPDIR` at the workspace, or raise the scratch, when the job sorts more than it can hold.

**`setup=` installs Python packages into the workspace, not system packages into the image.** The root
is read-only, so a package manager cannot run at all: `apk add git` answers `ERROR: Unable to lock
database: Read-only file system`, and `apt-get install` fails the same way. If the job needs `git`,
`make` or a compiler, that is a choice of `image=`, not something `setup=` can add.

**Nothing bounds the WORKSPACE, and `df` inside the box agrees with the host.** `memory_mb` bounds RAM
and the tmpfs mounts that are charged to it; the workspace is a host directory and is charged to your
disk. Measured under `memory_mb=128`: a cell writing a 400 MiB file to the workspace returns
`exit_code 0, fault=None` (`track_files` reports `fat.bin`), and `shutil.disk_usage("/workspace").free`
inside the box reports **110 GiB**, which is the host's free space, so a job that preflights its own
output size is told yes. With the default workspace the damage is temporary, since it is a temp
directory removed when the `Sandbox` closes. With `workspace=` it is not: measured, a 300 MiB file is
still there after close and the host's free space dropped by 300 MiB. No option here caps it, and
`max_output_bytes`/`timeout_s` do not help, so bound it outside the box: a workspace on a filesystem you
size (a quota, an LVM volume, a sized tmpfs you mount there yourself), and check what the last run left
before starting the next. This is the same shape as `nproc` and `df` reporting the host under a `cpus`
cap, one page down: the numbers a box reads describe the machine, not the box.

**`max_output_bytes` limits what you RECEIVE, not what the job costs.** Measured: past the cap the
output is discarded and the process keeps running to the end, so a marker file written after the noisy
part is there and `exit_code` is 0 with `truncated=True`. A runaway producer therefore runs until
`timeout_s`, and the two caps are per-stream, so a failure on stderr survives a flood on stdout.

**A JVM's heap and this scratch add up to less than the cap by luck, not by design.** The JVM takes
1/4 of the cgroup (measured: `MaxHeapSize 134217728` under `memory_mb=512`) and the scratch clamp
takes at most 1/2, and 3/4 fits. Write `-Xmx` at 3/4 of `memory_mb`, which people do, and the
composition breaks: neither side knows about the other, and `/dev/shm` is in the same budget with no
bound at all.

**`track_files` reports the workspace, and only the workspace.** A job whose product lands in `/tmp`
reports nothing changed while having produced output. Measured: writing `/workspace/a` and `/tmp/b`
in one call reports `['a']`.

**Nothing in `/tmp` survives a `snapshot`.** A tmpfs is on no layer, so a marker written to the
scratch is gone after `restore` while the workspace marker is there. A `setup=` that stages files in
`/tmp` loses them.

**matplotlib works and complains.** It falls back to a temporary `MPLCONFIGDIR` because `$HOME` is not
writable, so the figure is produced AND stderr carries `mkdir -p failed for path
/root/.config/matplotlib: [Errno 30] Read-only file system`. `exit_code == 0` is green for a run the
user will report as broken. Pass `env={"MPLCONFIGDIR": "/tmp"}`, which is what the MCP server already
does.

**Server images need three things, and each announces itself separately.** Measured on
`nginx:alpine`: `open("/run/nginx.pid") failed (30: Read-only file system)`, then
`chown(...) failed (1: Operation not permitted)`, then it serves.

```python
Sandbox(image="nginx:alpine",
        tmpfs={"/run": "1m", "/var/cache/nginx": "16m", "/var/log/nginx": "4m"},
        cap_drop=())   # CAP_CHOWN is in the default drop, and nginx chowns its cache
```

`cap_drop=()` **widens the default posture**, and it is the only recipe here that does: measured,
`CapEff` goes from `0000000000000000` to `00000110bd84efff`. Under `security_profile="untrusted"` it
does not, because the bundle wins over the option (`CapEff` stays zero even with `cap_drop=()`), so a
server image and that bundle are mutually exclusive today. Both facts are pinned by the posture test.

Name the REAL mountpoint: `/var/run` is a symlink to `/run` on Alpine, and a tmpfs at the alias
leaves the path the program opens untouched. And a server that refuses to run as root (postgres:
`initdb: error: cannot be run as root`) has no answer here yet, because this binding does not expose
kern's `--user`. Rust, .NET and anything else with a package cache want the same two places for the same
reason. `HOME` stays the caller's decision because a build cache in `/workspace` is a host directory
nothing bounds; point it at a `tmpfs={"/home": "512m"}` instead if you want it capped and thrown away
with the box.


**An enforced `pids` cap produces no fault, deliberately.** A refused `fork` returns `EAGAIN`, which a
program is allowed to catch and exit 0 on, so a contained fork bomb reads as a successful run.
Labelling that a sandbox fault would misreport a process that exited cleanly. The cap is still
enforced: on WSL2, `pids=32` blocked at 29 forks while `pids=256` let 120 through.

## Moved here from the README, because a first call does not need them

**Writable paths: `/workspace`, `/tmp` and `/dev/shm`.** The box root is read-only, so `/tmp` is a
64 MiB tmpfs the binding mounts for you. Without it two things break quietly: a write naming `/tmp`
fails with `EROFS`, and `tempfile` falls back to the current directory, putting scratch into your
persistent workspace. The bytes are charged to the box's own memory cgroup, so filling `/tmp` OOMs the
box and never the host disk. Resize with `tmpfs={"/tmp": "512m"}`, remove with `tmpfs={}`, or bind your
own directory at `/tmp`. Name the REAL mountpoint: `/var/run` is a symlink to `/run` on Alpine, and a
tmpfs at the alias leaves the path the program opens untouched.

**The bytecode route `deps_readonly` closes.** `run_code` mounts `.deps` read-only, so a cell cannot
change what the next cell imports. A `.pyc` is validated on the source's timestamp and size, so a cell
could rewrite a dependency's bytecode, leave the `.py` untouched, and the next `import` would run it -
invisibly to `result.files` and `list_files()`. The setup box compiles before the mount closes, so the
default costs nothing.

**A `tmpfs` that would COVER a `mounts` bind is refused**, since the bind's files would then be on the
host and invisible in the box. "Cover" is the mountpoint relation, not a string compare. The other
direction is legal: a bind at `/tmp` with `tmpfs={"/tmp/scratch": "8m"}` gives a persistent `/tmp` with
a bounded ephemeral subtree, and both halves work.

**The unit is required and a `tmpfs` target may not contain a `:`.** kern's CLI takes both spellings and
means the opposite of what you do: a bare `"64"` is 64 BYTES, `"0"` is UNLIMITED, and `["/scratch:9g"]`
mounts a size rather than a directory. All three are refused here, with the reason. A size larger than
`memory_mb` is refused too, because `df` would report it to a program that preflights against it.

**`/dev/shm` cannot be resized and can be replaced.** `tmpfs={"/dev/shm": ...}` is refused because it
would shadow the hardened `/dev`; its apparent size describes the HOST.
`mounts={host_dir: "/dev/shm"}` is accepted and works, at two costs: a plain directory swaps an
unbounded RAM path for an unbounded DISK one, and a file written there is still on the host after the
box dies.

## Found by running the flagship case: an agent that fixes its own code

**`setup=` runs under the same `memory_mb` as your cells, and a pip install needs more than a cell
does.** MEASURED with `setup="pip install pandas matplotlib"`: at `memory_mb=64` the setup box is
OOM-killed before any of your code runs, and `Sandbox.__enter__` raises `SandboxError: setup failed
(exit 137)` carrying kern's own OOM sentence; at 256 it succeeds. The cap that is right for a cell is
not necessarily right for the install that precedes it, so size the Sandbox for the setup and cap the
cells separately (below). A killed setup also leaves pip's `pip-unpack-*` directories in the workspace,
which is a host directory: remove them or start from a clean one.

**The cap is a property of the SESSION, not of a call.** `Sandbox.run_code` takes `timeout_s` but not
`memory_mb`; the module-level `kern.run_code` takes both, because it builds a one-shot Sandbox for you.
So an agent that reads `fault == "oom"` and wants to retry with more memory opens a NEW Sandbox on the
SAME `workspace=`, which is the cheap move rather than the expensive one: MEASURED on this host, the
first session paid 16.2 s for the pip install, and the second and third sessions on that workspace
started in 364 ms and 756 ms because `.deps` was already there and `setup=` could be omitted. The file
state persisting is what makes the retry cheap.
