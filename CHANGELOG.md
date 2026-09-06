# Changelog

**CLI stability.** As of v0.7.0 the command surface is stable: the verbs, their flags, and the
`--json` output shapes change incompatibly only on a **minor bump** (`0.8.0`+, since kern is `0.x`),
never on a patch, and only after a deprecation entry here at least one release earlier. `--json`
output is additive: new fields may appear, so consumers must **ignore unknown fields**; removing or
renaming one is the breaking change the minor-bump rule covers. A `cli_surface_is_frozen` test
snapshots the surface and fails the build on any undocumented change. Internal config-file keys may still evolve, but
scripts and SDKs written against the CLI can rely on it. Install the release binary with
`curl -fsSL https://raw.githubusercontent.com/getkern/kern/main/install.sh | sh`, or build from source
with `cargo install --git https://github.com/getkern/kern getkern --locked`. Full detail for any entry
is in the git history.

## Unreleased

### Fixed

- **`kern killall --help`, `kern down --help` and `kern logout --help` printed the whole 184-line
  reference.** The per-verb help matched the FIRST token of each command line, and those three are
  documented as the second half of a pair: `kill <name>... | killall`, `up ... / down`,
  `login ... / logout`. Nothing matched, and the fallback prints everything. The line's whole head is
  read now, with `<...>`, `[...]` and `(...)` removed so a placeholder cannot pose as a verb, which is
  why `volume <create|rm|edit|prune>` still declares only `volume`.

  The test meant to catch this named fifteen verbs by hand and none of the three were in it. It now
  reads the verb list out of the reference itself, all 51 of them.

- **Nine lines of `kern --help` sat outside the description column** (five one short, two one long,
  one three short, and `pod` twenty long because the line did not fit at all). `pod` is two lines now.
  No verb and no flag changed: the snapshot moved on whitespace and that one split, checked by
  extracting both sets and diffing them, 76 verbs and 83 flags either side.

## v0.9.1 - 2026-09-05

**This release is FASTER than v0.9.0 on a bare box start, and it took two wrong answers to get
there.** 2.300 ms against 2.346, faster in 21 of 24 paired batches, while keeping the OOM fix.

The first cut of that fix was 0.10 ms SLOWER, because it parked kern's supervisor in a sibling cgroup
to keep it out of the whole-group kill, and a cgroup `mkdir` costs 90.5 us on this host. Measuring it
properly showed the leaf is needed on exactly one path: a scope or managed unit where
`prepare_delegated_scope` did not move kern into a leaf of its own, so the supervisor's own cgroup is
the one being armed with `oom.group = 1`. Everywhere else `child` is freshly created, the supervisor
cannot be inside it, and moving it buys nothing.

So the leaf is now created only on that path. 0.165 ms back, 24 of 24 paired batches, and the
properties it defends were re-checked in both layouts on four hosts and four systemd versions (249,
252, 255, 257): the OOM message survives, `memory.max` and `pids.max` inside the box read the caps
exactly, and a wide cap still lets the same workload finish.


**Cut for one defect that the released binary has on most hosts.** `--egress-allow` in v0.9.0 could
put a proxy on a port nothing in the box could reach, and on three of the five hosts measured it did
not even get the port: `cannot bind 127.0.0.1:3128 in box: Address not available (99)`, then every
allowed domain refused with `Connection refused`. Reproduced on an Arduino UNO Q and a Jetson Orin
Nano with the shipped v0.9.0 aarch64 binary, byte for byte, against the same command answering
`403 Forbidden` from the proxy on this tree.

The cause is a race: the egress pump joins the box's network namespace and binds while the box's init
is still pivoting its root and has not yet raised the loopback. The pump now raises it itself and
refuses to serve if it cannot, so readiness means reachable rather than bound. Five hosts measured,
and neither the kernel version nor the privilege level explains which ones refuse the bind; only
`connect` fails on all of them, which is why the guard reads the interface instead of a syscall.

`scripts/acceptance-matrix.sh` now exercises this, and says out loud when it CANNOT: on a host that
accepts a bind on a down loopback, v0.9.0 passes the case exactly as the fix does, so the matrix
prints that instead of a green tick. Verified in both directions on hardware, red on the released
binary and green on this one, on the board where the defect lives.

Also in this release: kern's progress output no longer contaminates a pipe, a cgroup probe no longer
prints systemd's bus error onto the box's stderr, and three diagnostics that lacked the `kern: `
prefix were invisible to anything reading kern's output. Detail below.

### Fixed

- **The MCP server offered a language and then refused it.** The `run_code` tool schema advertised
  `["python", "bash", "sh", "node"]` while the guard in `_run_code` compared against a second,
  hand-written `("python", "bash", "node")`. A model reading the schema sent `sh` and got
  `unsupported language: 'sh'`, and the message did not say what would have worked.

  It mattered most where it was least visible. On an image with neither python nor bash, `sh` is the
  only shell there is, so `KERN_MCP_IMAGE=alpine` gave a server whose own schema promised something
  it could not do. The guard now IS the schema's list rather than a copy of it, and a refusal names
  the accepted values. Found by driving the server sixty times in a row, not by reading either line.

  Three tests pin it: the guard must be the same object as the schema's enum, every language
  `Sandbox.run_code` accepts must be offered by the server, and `sh` must be among them. Mutation
  checked by reverting the constant to the old triple, which turns all three red.


**`kern-sandbox` 0.1.41** is a documentation release: no code changed from 0.1.40. PyPI and npm fix a
package's description at upload time, so the only way to correct a landing page is to publish again.

The two package READMEs had grown into reference manuals, 521 and 387 lines, and are now 384 and 309
with nothing deleted. The operational tail moved to `SANDBOX-NOTES.md` beside each binding, the way
`LANGCHAIN-SHELL.md` already handles the shell policy: scratch that does not survive a call,
toolchains that need `HOME`, a `df` and an `nproc` that describe the host, output discarded past the
cap while the job runs on, matplotlib rendering and complaining anyway. None of it is needed for a
first call and all of it cost somebody an afternoon.

Both pages now document `code_stderr`/`codeStderr` and `runtime_notes`/`runtimeNotes`, which shipped
in 0.1.40 with no page saying so, and the LangChain section names the two vocabularies the policy
accepts.


**`kern-sandbox` 0.1.40** answers an external audit of the SDK, the pi extension and the LangChain
integration. Four findings were reported; one was already closed in 0.1.39, and measuring the second
found a worse defect underneath it than the one described.
The **runtime** changed too, so this one needs a tag: mount-posture fixes in `kern-isolation`, the OOM
message on hosts where it never printed, and one new `kern box` flag. The CLI change is additive
(`--shm-size`), which the stability policy above allows on a patch.

### Fixed

- **The egress pump could report itself ready on a box nothing could reach.** With `--egress-allow`,
  the pump joins the box's net namespace and listens on `127.0.0.1:3128`. It is handed the box's pid
  the instant `clone` returns, while the box's init is still pivoting its root and has not yet raised
  the loopback, so the two race. An audit saw the bind fail with `EADDRNOTAVAIL` and every allowed
  domain refused with it.

  What a down loopback does turns out to depend on the kernel, and one C probe run on both hosts
  settled it. On 6.12.8+ the bind is refused, as reported. On 7.0.0 it is not: **`bind` and `listen`
  both succeed**, and only the box's own `connect` fails, with `ENETUNREACH`. The second row is the
  worse one, because the pump took a port and wrote the readiness byte added last release to make
  exactly this failure visible, then the launcher started the workload against a proxy no packet could
  reach. A false green in the signal built to prevent one.

  The pump now raises the loopback itself before binding, which removes the dependency on the init's
  progress instead of narrowing the window, and treats a loopback it cannot raise as fatal. Readiness
  means reachable, not bound - which is also what makes one guard right on both kernels, rather than
  two cases handled separately. `bring_loopback_up` is idempotent and reports success for a loopback the
  init had already raised, so either order is a success. A test in a fresh net namespace pins the
  asymmetry, with the unreachable state asserted first as its own positive control.

- **`kern_execution_policy(cap_drop=("ALL",))` was disabling the drop it asked for.** Found in a
  clean-code pass over the aliases added the same day, not by a test. `Sandbox.cap_drop` is a sequence
  of capability names and the policy's `drop_all_capabilities` is a bool, and the converter compared
  the value against the STRING `"ALL"`. A tuple is not that string, so `("ALL",)` - which is
  `Sandbox`'s own default and the spelling anyone copying from the docs writes - produced
  `drop_all_capabilities=False`. A narrower set like `("SYS_ADMIN", "NET_RAW")` collapsed the same way,
  its narrowing discarded in silence.

  A convenience alias that quietly weakens a boundary is worse than no alias. The two values with an
  exact image convert (`("ALL",)` and `()`, in every spelling), and everything else raises, naming the
  field to set directly. `None` now passes through on `memory_mb` and `pids`, where it means "no cap"
  on both sides and used to raise a message about `int()`.

- **kern's progress output no longer contaminates a pipe.** Nineteen lines (`-> resolving …`,
  `-> layer …`, `✓ pulled …`, per-service compose bring-up, port publishing) were written with a bare
  `eprintln!` and so went out on kern's stderr whatever was reading it. Under the SDK that stream is the
  box's stderr, and a validation run on an uncached image came back with six of them in front of the
  program's own output, inside a LangChain tool result. They are now written through
  `kern_common::progress!` (and a deliberate twin in the libc-only `kern-isolation`), which prints only
  when stderr is a terminal: the rule the `kern box` status panel already followed and the pull path
  never did.

  A terminal is unchanged, verified as a positive control rather than assumed. Errors, warnings and
  `kern: note:` advice are untouched, because a pipe is exactly where those must still arrive.

  `scripts/progress-is-tty-gated.py` is the reason this stays true. Converting the sites by hand found
  fourteen and missed five: one whose format string sat on the following line, and four in files nobody
  thought of as progress, including the port-publishing path that every `-p` box hits. The gate reads
  the whole macro call, covers both crates, and self-tests through `gates-selftest.py`.

  These are NOT two layers over one problem, and calling the prefix classifier a "consumer-side
  complement" to this gate overstated the arrangement. They cover disjoint sets. Progress must not
  reach a pipe, so the TTY gate closes it at the source and the classifier never sees one. A warning
  MUST reach a pipe, which is what a warning is for, so the gate deliberately passes it and the prefix
  classifier is the ONLY mechanism for that class. The auditor's objection, that the next diagnostic
  will not be in the list, is therefore unmitigated for warnings rather than half-mitigated.

  What bounds it is that kern's own lines are required to carry `kern: `, and three that did not were
  fixed in this release (below). A file descriptor per writer is the only thing that would remove the
  convention entirely; it is not built, and the honest statement is that this class rests on a
  convention plus a gate over the modules an SDK caller's stderr is made of.

- **Three diagnostics were invisible to the SDK because they lacked the `kern: ` prefix.** A bare
  `warning: bound 0.0.0.0 - box port N is reachable from the network` (every `-p 0.0.0.0` box), a bare
  `note: pulled linux/arm64 ...` (every cross-architecture pull) and a bare `warning:` from `kern
  build` reached `code_stderr` as though the workload had printed them. Five `kern compose:` lines had
  a near-miss prefix that matches neither the benign list nor the failure marker, and are now
  `kern: warning: compose:` / `kern: note: compose:`. Found by the gate above once it was scoped to
  whole modules instead of to a set of leading markers.

- **A cgroup probe printed systemd's bus error onto kern's stderr.** To find out whether a delegated
  slice can be made, kern runs `systemd-run ... -- true` and reads the answer from the exit status. Its
  stderr was inherited. On a host with the systemd tools installed but not booted (a container, a WSL
  session, plain root), that probe answers "no" and prints `System has not been booted with systemd as
  init system` and `Failed to connect to bus`, which travelled into a LangChain tool result where a
  model read a line about dbus as though its own code had produced it. `--quiet` does not cover it: it
  suppresses systemd-run's info messages, not the bus error. Both streams are now null, as the other
  three `systemd-run` call sites already did. The verdict is unchanged; it was never in the output.

- **kern's diagnostics no longer land in a model's context.** kern and the workload share one stderr,
  so `kern: note:` and `kern: warning:` lines arrive interleaved with the program's output. The audit
  found them inside a LangChain tool result, where they cost context and read like errors the code
  produced. `result.code_stderr` / `result.codeStderr` is `stderr` without them and is what the
  LangChain tool and the MCP server now render; `runtime_notes` / `runtimeNotes` holds exactly what
  was removed, and `stderr` still holds every byte in its original order. The classifier is one
  constant per binding, shared with the startup-failure heuristic that used a second copy of the same
  list. A workload that forges one of kern's prefixes moves its own line out of what the model reads
  and cannot inject text into it.

- **`kern_execution_policy` accepts `Sandbox`'s vocabulary.** `command_timeout` and `memory_bytes` are
  langchain's own field names, inherited from the base class this policy subclasses, so they cannot be
  renamed without it ceasing to be a drop-in peer of `DockerExecutionPolicy`. `timeout_s=` therefore
  raised `TypeError: unexpected keyword argument` with nothing in the message naming the spelling that
  works. Both are accepted now, the unit converting with the name (`memory_mb=256` becomes
  `memory_bytes=268435456`); passing both halves of a pair is refused rather than silently resolved,
  and an unknown name is rejected with the accepted spelling in the message.

### Changed

- **`deps_readonly` now defaults to TRUE** in both bindings, so `run_code` mounts what `setup=`
  installed read-only and a cell cannot change what the next cell in the same session imports. The
  route it closes is bytecode: a `.pyc` is validated on the source's timestamp and size, so a cell
  could rewrite `.deps/.../mylib.pyc`, re-paste the legitimate 16-byte header, leave the `.py` alone,
  and the next `import` ran it. Invisible to `result.files` and to `list_files()`.

  **What breaks:** a workload that writes into `.deps` at RUN time now gets `EROFS`. `deps_readonly=False`
  restores the old behaviour. A run-time `pip install` fails first for the network, which is off
  outside `setup=`, not for the mount.

  It costs nothing at run time: the setup box compiles the bytecode before the mount closes. Without
  that step a session whose setup skipped compilation paid +40 ms on every call, for its whole life
  (250 ms/call against 290, measured on `requests`).

- **A timeout now reports `exit_code = 137` in Python, not `-9`.** Node already did; a shell, kern's
  own CLI and docker all do. A caller branching on `137` saw the timeout in one binding and missed it
  in the other. Ordinary exit codes are untouched.

- **`integrations/pi` declares `engines: node >= 22`.** `pi`'s package manager imports `globSync` from
  `node:fs`, which landed in 22, so on Node 20 the extension died at import with a `SyntaxError`
  naming a file inside `pi-coding-agent`. Measured: 20.18.1 fails, 22.11.0 runs all 165 assertions.

### Added

- **`kern box --shm-size SIZE`**, for a workload that needs `/dev/shm` sized differently from
  `--memory`.

- **`kern-sandbox` (Python and Node): `prewarm=N`** keeps N boxes started in advance, so `run_code`
  costs ~1 ms instead of ~39 without giving up the fresh-box guarantee: a prewarmed box serves exactly
  one cell and is then destroyed. Measured over ssh: 37.8 ms per call before, 1.6 ms after.

  A slot refills in ~70 ms, so N is a burst budget rather than a throughput setting: N back-to-back
  calls run at ~1 ms and the rest fall back until the pool catches up. A call whose posture differs
  from the pooled box's (network, mounts, env, profiles, or a deadline longer than the box's remaining
  backstop) is never served from the pool, and a streaming call falls back to the cold path so it can
  stream for real. One observable does differ: the interpreter is older than the call, so a cell
  reading its own start time sees ~0 s cold and up to five minutes warm.

  Default `0` in the SDK, because holding a booted interpreter per slot is the caller's resource
  decision. Default `1` in `kern-mcp` (`KERN_MCP_PREWARM`), where the session already holds a box for
  its whole life.

- **Every box gets a writable `/tmp`**, 64 MiB of tmpfs, charged to the box's own memory cap.
  `security_profile="untrusted"` gets none, deliberately. Nothing in `/tmp` survives a call or a
  `snapshot`.

### Fixed (runtime)

- **The OOM message never printed when kern runs as root, or on a host with no systemd** - which is
  where the cap is most likely to be the only thing between a workload and the machine. A box killed
  by its cap exited 137 with an empty screen.

  Two causes. The counter was read by walking KERN's cgroup ancestors, which answers for the box only
  when the two share one: on a root VPS kern sits under `user.slice` and the box under `system.slice`,
  whose only common ancestor never exposes `memory.events`. And on the direct-cap path the supervisor
  sat INSIDE the box's cgroup, so `memory.oom.group` killed the process that had to report the kill.
  The counter is now read from where the box is, and the supervisor sits in a sibling leaf while the
  workload joins the capped cgroup itself.

  Verified on four hosts: WSL2 with no systemd, a root VPS, a non-root Jetson and one desktop. On all
  four a clean exit still says nothing, a SIGKILL that is not the cap is not blamed on it, and
  `--memory` still binds on `box` and on `run`.

- **`--egress-allow` could leave a box with NO outbound access and say nothing.** The in-box proxy port
  is opened by a helper that joins the box netns; when that bind failed, the box started anyway with
  `http_proxy` pointing at a dead port, so every request - including to the allowed domains - failed
  with `Connection refused` naming neither the cause nor the flag. The helper now confirms it is
  listening before the workload runs, and the box is refused with a message naming the flag if it
  cannot.

- **The registry recorded the supervisor's cgroup for every box**, and `kern stop` writes
  `cgroup.kill` into the path it records, so a stop would have killed the reporter and left the box
  running. It now records the directory kern created rather than one derived from the child's `/proc`
  entry mid-fork.

- **A `-v` volume is now mounted `nosuid`, `/workspace` included.** Defence in depth: `PR_SET_NO_NEW_PRIVS`
  is armed before the workload runs, which already makes the setuid bit inert process-wide, so a failed
  `nosuid` remount is never fatal. A `:ro` volume still fails hard, because read-only is a contract the
  caller asked for and nothing else provides it.

- **`/dev/shm` now reports the size the box actually has.** It was mounted with no `size=`, so
  `statvfs` reported half the HOST's RAM: a box held at 512 MiB was telling every workload it had
  15.6 GB, which is the number Postgres, Chromium and a PyTorch DataLoader size buffers from. It now
  carries the cap already enforced. A box with no cap enforced anywhere keeps the unsized mount,
  because there is no honest number to put there.

### Fixed (kern-sandbox)

- **A warm interpreter could not import anything the IMAGE ships**, and the shipped `kernel()` had
  that defect since it landed. The driver started as `python3 -S`, which skips `site` and therefore
  `site-packages`, so `import numpy` worked on a cold `run_code` and raised `ModuleNotFoundError` in a
  kernel cell. What hid it: `setup=` installs into `.deps`, which is on `PYTHONPATH` either way, so
  only cells relying on the image could not import. `-S` is dropped in both paths and both bindings.

  A custom image that ships `.pth` files will now run their `import` lines at interpreter start, and
  for a prewarmed box that happens at pool-fill time. The default image ships none.

- **`kern-sandbox` 0.1.36 on npm could not be installed at all.** Its `package.json` declared a
  dependency on itself via a local tarball path, so `npm install` failed with `ENOENT`. Fixed in
  0.1.37; 0.1.36 is deprecated on npm with a message naming it. The PyPI package of that version is
  unaffected.

- **`memory_mb` bounds the cgroup, not the workload's usable memory**, and the docs now say so: a
  tmpfs, `/dev/shm` and the page cache are charged to the same cap.

## Earlier releases

v0.9.0 back to v0.7.0 are in **[docs/CHANGELOG-HISTORY.md](docs/CHANGELOG-HISTORY.md)**.
