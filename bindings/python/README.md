# kern-sandbox

**Run AI-generated code in a real sandbox, one fresh box per call, in about 4 ms.**

`kern-sandbox` is the Python binding for **[kern](https://getkern.dev)**: a rootless,
kernel-enforced sandbox out of one static binary, with no daemon, no VM and no cloud. An agent's
tool-call, a model's generated snippet, a notebook cell, a CI step: code that runs before anyone
reads it gets its own box, and the box is thrown away after.

```bash
pip install kern-sandbox
```

**It needs the `kern` binary**, because the isolation is the binary's and this package is the API in
front of it. One line, no toolchain, and `$KERN_BIN` overrides where it looks:

```bash
curl -fsSL https://raw.githubusercontent.com/getkern/kern/main/install.sh | sh
```

```python
import kern_sandbox as kern

r = kern.run_code("import sys; print(sys.version)")
print(r.stdout, r.success)
```

Network off, memory and PID caps the kernel enforces, capabilities dropped, a deny-by-default seccomp
allowlist, and a wall-clock deadline applied from **outside** the box, so code that hangs cannot
outlive it. Node and TypeScript get the same binding on npm: [`kern-sandbox`](https://www.npmjs.com/package/kern-sandbox)
(the MCP server below is this package's).

## Your loop reads a field, not a stack trace

A timeout, an OOM-kill, a blocked syscall or a missing interpreter each arrive as a **typed field on
the result**, beside stdout and the exit code. The agent branches on a value instead of parsing a
traceback to work out whether the sandbox stopped the run or the code did.

```python
r = kern.run_code("while True: pass", timeout_s=5)
r.fault.type      # 'timeout'      the sandbox stopped it
r.success         # False
```

Every call returns an `ExecutionResult`:

```python
@dataclass
class ExecutionResult:
    stdout: str
    stderr: str
    exit_code: int
    duration_ms: int
    fault: SandboxFault | None   # set ONLY when the SANDBOX acted
    files: list[FileInfo]        # workspace files created or modified this step (.deps excluded)
    results: list[Result]        # rich mime-typed values: last expression, display(), matplotlib
    truncated: bool              # output hit max_output_bytes and the overflow was discarded
    success: bool                # exit_code == 0 AND fault is None
    code_stderr: str             # stderr minus kern's own note:/warning: lines - feed THIS to a model
    runtime_notes: list[str]     # the complement: the lines kern wrote about itself
```

**A Python exception in the code is NOT a fault.** That is `exit_code != 0`, a traceback in `stderr`,
and `fault is None`, because the code ran and the sandbox did nothing. `fault` is set only when the
sandbox acted:

| `fault.type` | what happened |
|---|---|
| `timeout` | the call exceeded `timeout_s`; the binding owns that deadline |
| `oom` | the kernel's OOM killer took the box against its own memory cap. Read from a descriptor the code in the box cannot write, so it is an observation and not a guess from the exit code |
| `killed` | SIGKILL with **no** OOM reported: an external kill (`kern stop`, a signal, the host out of memory), or a cap that did not bind here. A cap being set is not evidence that memory is what killed the box |
| `escape_blocked` | a syscall the seccomp filter refused (SIGSYS) |
| `exec_failed` | the box started, the command did not exist in the image; the message names both |
| `startup_failed` | your `timeout_s` fired while kern was still BUILDING the box, so the code never ran. A longer timeout does not help: a bind source on a dead NFS export does this |

A box that fails to **start** raises `SandboxError` instead, because the code never ran.

`stderr` is one stream shared by kern and your code, so a note about an undelegated cgroup arrives
interleaved with the program's own output. Right for a human at a terminal, wrong for anything that
puts `stderr` into a prompt. `code_stderr` is the same string without kern's own lines, `runtime_notes`
holds exactly what was taken out, and `stderr` still holds both in order. The LangChain tool and the
MCP server use `code_stderr`.

## Use it from an MCP client (Cursor, Claude Desktop, anything that speaks MCP)

The package ships **`kern-mcp`**, a dependency-free
[Model Context Protocol](https://modelcontextprotocol.io) stdio server: the model writes code, kern
runs it on your machine, and charts come back as images it can see.

**Where it runs matters, because kern is Linux-only.** The stdio transport spawns the server where the
CLIENT runs, so the config below is for a Linux client. From Windows or macOS it is one hop and still
one line (`"command": "wsl"`, or `"command": "ssh"` to a VM or a board), both in
[docs/MCP.md](https://github.com/getkern/kern/blob/main/docs/MCP.md).

```json
{
  "mcpServers": {
    "kern": {
      "command": "kern-mcp",
      "env": { "KERN_MCP_SETUP": "pip install numpy pandas matplotlib" }
    }
  }
}
```

Tools: `run_code` (python/bash, and node on an image that has it), `write_file`, `read_file`,
`list_files`. File state persists across calls; each call is a fresh, network-off box. The tool schema
names the configured image and says which interpreters it provides, so the model is not left to infer
that from the enum.

| Env var | Default | What it does |
|---|---|---|
| `KERN_MCP_IMAGE` | `python:3.12-slim` | OCI image the boxes run in |
| `KERN_MCP_SETUP` | (none) | one-time `pip install ...`, the ONLY network-on moment |
| `KERN_MCP_MEMORY_MB` | `1024` | hard RAM cap per box; `0` sends no flag, so a `vcpu:` profile's own `memory=` applies |
| `KERN_MCP_TIMEOUT` | `60` | per-call wall-clock deadline |
| `KERN_MCP_WORKSPACE` | temp dir | persist file state at this path |
| `KERN_MCP_PROFILES` | (none) | attach `kern.toml` profiles, e.g. `vcpu:heavy,vgpio:sensors`: the only way to grant an edge agent a hardware device |
| `KERN_MCP_KERNEL` | off | `1` routes Python through one warm interpreter: state persists, each call is sub-millisecond. The one case where "a fresh box per call" stops being true, and the tool description says so to the model |
| `KERN_MCP_QUIET` | on | `0` restores kern's non-fatal notes |
| `KERN_MCP_TMPFS_MB` | `64` | scratch at `/tmp`, charged to the box's own memory cap; `0` removes it and puts `/tmp` back inside the read-only root |

**Why local rather than hosted.** No account, no API key, no round-trip: the model's code runs on
your machine, and it works air-gapped. Full reference:
[docs/MCP.md](https://github.com/getkern/kern/blob/main/docs/MCP.md).

## The model: file state persists, processes do not

**File state persists** through a `/workspace` directory shared into every box: write a file in one
call, read it in the next. **Processes are ephemeral**, so `x = 40` in one call is gone in the next.
That is deliberate: it keeps the density of hundreds of ephemeral boxes instead of hundreds of
resident interpreters.

When you do want in-memory state, open a `kernel()`: one warm interpreter in a long-lived box,
per-cell cost **sub-millisecond** instead of a ~12 ms CPython boot, with the explicit trade that cells
share one process and one box.

```python
with kern.Sandbox() as sbx, sbx.kernel() as k:
    k.run_code("import numpy as np; a = np.arange(1_000_000)")
    print(k.run_code("a.sum()").results[0].text)     # 499999500000
```

A refused mount raises `MountRefused` rather than the generic `SandboxError`, so a caller can tell
"this sandbox will not do that" from "the sandbox broke".

## Prewarming: a box ready before the call arrives

`prewarm=N` keeps N boxes started in advance, each holding a booted interpreter that has run nothing,
and refills on a worker thread while your agent thinks. Measured on `python:3.12-slim`:

| | first call | p50 within the burst |
|---|---:|---:|
| default | 30.9 ms | 14.2 ms |
| `prewarm=4` | 0.9 ms | **0.8 ms** |

**The pool covers a burst, not a rate**, and it fills on that worker thread: measured, four calls with
`prewarm=4` read a p50 of 0.6 ms and twenty read 13.6 ms, which is the default. Constructing and calling
immediately reads 13.7 ms until the boxes have started.

Each prewarmed box still serves ONE call and is thrown away, so the isolation is unchanged: only the
moment of creation moves. That is the difference from `kernel()`, which shares one process across cells
and says so. The pool key includes the image, the caps and the profiles, so a session never receives a
box built for another one.

```python
with kern.Sandbox(image="python:3.12-slim", prewarm=4) as sbx:
    r = sbx.run_code("print(1)")     # served from the pool
```

## Run pi's coding tools in a box

[`integrations/pi`](https://github.com/getkern/kern/tree/main/integrations/pi) is an extension for
[pi](https://github.com/earendil-works/pi) that routes its built-in `bash`, `read`, `write`, `edit`,
`ls`, `grep` and `find` tools through this SDK into a kern box. Your working directory is mounted at
`/workspace`, so edits write through to the host and everything else a command touches dies with the
box. pi's default posture is no sandbox at all: it runs as the user who launched it.

The two halves are not confined by the same thing, and the extension's README says which is which:
`bash` runs INSIDE the box, while `read` and the staging half of `write` are host filesystem calls
guarded by `O_NOFOLLOW` plus a `/proc/self/fd` containment check. Needs Linux, the `kern` binary, and
Node 22 or newer.

## Charts and rich results, without a Jupyter kernel

`run_code` captures mime-typed values into `result.results` the way a notebook cell does: the **last
bare expression**, every **`display(obj)`**, and **every open matplotlib figure automatically**, with
no `savefig`. Accessors: `.png`, `.jpeg`, `.html`, `.svg`, `.markdown`, `.json`, `.text`.

```python
with kern.Sandbox(setup="pip install pandas matplotlib") as sbx:
    sbx.write_file("data.csv", "a,b\n1,2\n3,4\n")
    r = sbx.run_code("import pandas as pd; pd.read_csv('data.csv').describe()")
    r.results[0].html          # the DataFrame as an HTML table
```

Capture never touches `stdout`, `stderr` or `exit_code`. Pass `on_stdout` / `on_stderr` to stream as
output arrives (best-effort: a slow callback drops chunks rather than stalling the box).

## Safe by default

A bare `Sandbox()` has no network, no host mounts, seccomp on, dangerous capabilities dropped and a
**mandatory** finite timeout. Every relaxation is a named argument:

```python
Sandbox(
    image="python:3.12-slim",   # OCI image
    setup="pip install pandas", # the ONLY network window: a separate net-on box; run_code is net-off
    workspace=None,             # None -> temp dir, deleted on exit; a path -> persists
    memory_mb=512,
    cpus=None,                  # CPU cap in cores (e.g. 1.5); None = uncapped
    pids=256,                   # fork-bomb ceiling
    timeout_s=30,               # MANDATORY per-call wall-clock limit
    network=False,              # RELAXES ISOLATION: True shares the host network for every run
    mounts=None,                # {host_src: box_target}; sensitive sources refused even if asked
    tmpfs=None,                 # None -> 64 MiB of scratch at /tmp; {} -> none; {"/tmp": "512m"}
    profiles=None,              # kern.toml profiles: ["vcpu:heavy", "vgpio:leds", "vdisk:scratch"]
    max_output_bytes=64 << 20,  # cap on captured stdout/stderr EACH; result.truncated on overflow
    deps_readonly=True,         # run_code cannot modify setup= deps; False re-opens it
    security_profile=None,      # "untrusted" = seccomp allowlist + cap-drop ALL + read-only root
    apparmor=None,              # a pre-loaded AppArmor profile; kern fails CLOSED if it is not loaded
    require_limits=False,       # True = refuse to start unless memory/pids caps are enforced
    cap_drop=("ALL",),          # default drops ALL; pass () only if the box must bind a port < 1024
)
```

**Mounts over sensitive sources** (`/`, `/etc`, `$HOME`, the docker socket) are refused even if you ask
for them, and so is a `tmpfs` that would cover a `mounts` bind.

**`setup=` output is read-only to your code.** A cell cannot change what the next cell imports.
`deps_readonly=False` reopens it, and a write then gets `EROFS` rather than failing silently.

**`egress_allow` is the middle setting, and the one an agent usually wants.** `network=False` gives the
run phase no network, `network=True` gives it the host's, and an allowlist gives it a named few:

```python
kern.Sandbox(egress_allow=["pypi.org", "files.pythonhosted.org"])
```

The box stays in its own network namespace and reaches the internet only through kern's filtering
proxy. Mutually exclusive with `network=True`. Otherwise the network is on **only** during `setup=`, in
a separate box that dies when setup ends.

It is a **route-level** boundary, not a set of proxy variables a program can ignore, and that cuts both
ways. Measured inside an `egress_allow` box: a raw socket to an IP returns `ENETUNREACH`, DNS does not
resolve at all, and an HTTP request to a domain outside the list comes back `Tunnel connection failed:
403 Forbidden`, while the same socket under `network=True` connects. Nothing leaves except through the
proxy. So a client that does not speak to an HTTP proxy has **no path out**: a Postgres, MySQL or Redis
connection under `egress_allow` fails to resolve its host, and that is the design rather than a bug. If
the job needs a database, the network setting for it today is `network=True`.

**`memory_mb` bounds the cgroup, not the workload's usable memory**, and `/dev/shm` is not bounded at
all: measured, 200 MiB written there under `memory_mb=128` OOM-kills the box, while the same 200 MiB to
`/tmp` returns `ENOSPC` and the box lives. Python's `multiprocessing` uses `/dev/shm` by default, so
this is not a corner.

**Not capped: the workspace on disk.** It is a host directory, and file state persisting is the point.
A cell writing in chunks put 400 MB on the host under `memory_mb=128`, because a memory cap only stops
the version that builds the payload in RAM first. Point `workspace=` at a filesystem you have bounded.

**Resource profiles** attach slices defined once in `~/.config/kern/kern.toml`: `vcpu:` (CPU and
memory), `vdisk:` (a size-capped scratch disk), `vgpio:` (a device set, the **only** way to give a box
hardware). An explicit flag beats a profile, so pass `memory_mb=None` to let a `vcpu:` profile's own
`memory=` apply.

**The rest is in [SANDBOX-NOTES.md](https://github.com/getkern/kern/blob/main/bindings/python/SANDBOX-NOTES.md)**,
where every entry is a measured surprise and none is needed for a first call: which paths are writable
and why `/tmp` is one of them, the bytecode route `deps_readonly` closes, the size rules `tmpfs=` and
`mounts=` enforce, scratch that does not survive a call, toolchains that need `HOME`, a `df` that
describes the host, and the three things a server image asks for.

## API

- `kern.run_code(code, **kwargs)`, one-shot: a throwaway `Sandbox` under the hood.
- `Sandbox(...).run_code(code, language="python"|"bash"|"sh"|"node")` on the session workspace. The
  enum is what the runner accepts, **not a promise about the image**: the default `python:3.12-slim`
  ships `python`, `bash` and `sh` and no `node`, and asking for a missing interpreter returns an
  `exec_failed` fault naming the binary and the image. **`bash` runs bash and `sh` runs the POSIX
  shell**, which are different languages: `[[ ]]`, arrays and `pipefail` are bash. Alpine has no bash
  at all, so ask for `sh` where the image may not carry one.
- `Sandbox(...).run(argv_list)`, an arbitrary command (an **argv list**, never a shell string).
- `Sandbox(...).write_file(path, data)` / `.read_file(path)` / `.list_files(subdir="")`, workspace
  I/O, confined to `/workspace`, `..`-safe, every path component opened `O_NOFOLLOW`, opened
  `O_NONBLOCK`, and a descriptor that is not a REGULAR file is refused. A symlink is not the only
  thing a box can leave at a name: `mkfifo out.png` used to make `read_file("out.png")` wait for a
  writer that never came, with no timeout, so the box chose how long the host's call took. The flag
  alone would have been worse, since a non-blocking read of a writer-less FIFO returns zero bytes and
  the call would have reported an empty file.
- `Sandbox(...).snapshot(dest)` / `.restore(src)`, a portable `.tar.gz` FILESYSTEM checkpoint of the
  workspace. `restore` refuses absolute, `..` and symlink members.

## Use it from LangChain

```bash
pip install 'kern-sandbox[langchain]'
```

```python
from kern_sandbox.langchain import kern_code_tool

tool = kern_code_tool(memory_mb=512, timeout_s=30)
agent = create_agent(model, [tool])
```

One session, so a file written by one call is there for the next, and each call still runs in a fresh
box. What comes back is written for a model to act on: stdout, the value of a trailing expression, and
the traceback when the code raises. A sandbox fault is labelled (`[sandbox: timeout]`, `oom`,
`escape_blocked`) so the model does not debug code that was killed for asking for 4 GB.

Everything a box prints is untrusted text on its way into a context window, so the rendering strips
terminal escapes and neutralises that framing wherever the **code** produced it: a cell printing
`[sandbox: oom]` would otherwise claim, byte for byte, that the sandbox killed it. Ordinary prompt
injection is **not** filtered and cannot be at this layer.

There is also a **shell execution policy** for LangChain's shell middleware, the long-lived-session
shape rather than one box per call, with its own page:
[LANGCHAIN-SHELL.md](https://github.com/getkern/kern/blob/main/bindings/python/LANGCHAIN-SHELL.md).

## Performance

One x86_64 desktop (i7-14700KF, Linux 7.0.0, rootless, cgroup delegated), `python:3.12-slim`, p50 over
25 calls after a discarded warm-up. Your hardware will differ: measure and claim your own number.

| call (p50) | kern-sandbox | docker |
|---|---|---|
| `run(["true"])`, bare box | **3.9 ms** | |
| `run_code("print(1)")`, plus the CPython start | **14.3 ms** | ~290 ms |

`run_code` runs *Python*, so it pays the interpreter boot on top of the box: that is a Python cost, not
kern's, and it is why 14.3 rather than 3.9.

**The host and the image are part of the claim.** The same call reads ~40 ms on WSL2 and ~17 ms on
`python:3.12-alpine`, whose interpreter starts slower. Quote the row that matches yours.

**Concurrency:** 100 concurrent `run_code` calls on one `Sandbox` finish in **0.30 s** wall clock,
100/100, no leaked boxes. The 211 ms per-call p50 in that run is queueing, not latency.

Method, the other runtimes, and why `enforce_limits=False` is not a speed knob:
[BENCHMARKS.md](https://github.com/getkern/kern/blob/main/BENCHMARKS.md).

## Threat model (honest)

kern is a **kernel-boundary** sandbox for **your own or semi-trusted** code. The default seccomp
filter is a deny-by-default allowlist (moby's own default minus kern's 35 escape syscalls): suitable
for agent-generated code, **not** a hard boundary against deliberately hostile multi-tenant code. For
that, use a microVM (Firecracker, Kata) or gVisor. `security_profile="untrusted"` bundles the
allowlist with `--cap-drop ALL` and `--read-only`. The full statement is in
[SECURITY.md](https://github.com/getkern/kern/blob/main/SECURITY.md).

## Requirements

The `kern` binary on `PATH` (or `$KERN_BIN`). A Linux kernel with unprivileged user namespaces and
cgroup v2; on Windows it runs under WSL2. Python 3.9+.

**On a Mac this package installs but cannot run**, and it says so rather than looking for a download
that does not exist: kern is Linux-only, because macOS has no namespaces and no cgroups. Run it inside
a Linux VM (colima, Lima, OrbStack, UTM). Verified on Apple Silicon with an Ubuntu 24.04 guest.
[Install notes](https://github.com/getkern/kern/blob/main/docs/INSTALL.md).

## License

Apache-2.0.
