# Changelog

**CLI stability.** Since v0.7.0 the verbs, their flags and the `--json` shapes change incompatibly
only on a minor bump, never on a patch, and only after a deprecation entry here one release earlier.
`--json` is additive, so consumers must ignore unknown fields. A `cli_surface_is_frozen` test fails
the build on any undocumented change. Full detail for any entry is in the git history.

## Unreleased

### Fixed

- **`kern killall --help`, `kern down --help` and `kern logout --help` printed the whole 184-line
  reference.** The per-verb match read only the first token of each line, and those three are
  documented as the second half of a pair. The test that missed them named fifteen verbs by hand; it
  now reads the list out of the reference, all 51.
- **Nine `kern --help` lines sat outside the description column**, `pod` by twenty because it did not
  fit; `pod` is two lines now. 76 verbs and 83 flags either side, checked by diffing both sets.
- **A damaged image-cache entry was repaired in silence under an SDK.** v0.9.1 gated kern's progress
  on a terminal and took the two repair lines with it, so in a pipe a cached image with no usable
  rootfs, or with no config, was re-fetched with nothing said. They are `kern: note:` now and reach a
  pipe; the ordinary "not cached, pulling once" stays gated. Found by
  `pentest/pentest-cache-edge.sh`, which asserts kern names the missing part.

## v0.9.1 - 2026-09-05

**Faster than v0.9.0 on a bare box start**, 2.300 ms against 2.346 in 21 of 24 paired batches, with
the OOM fix kept. The supervisor's sibling cgroup is created only where it is needed, a scope or
managed unit whose own cgroup is the one armed with `oom.group`: 0.165 ms back, 24 of 24, re-checked
in both layouts on four hosts and four systemd versions (249, 252, 255, 257).

**Cut for one defect the released binary had on most hosts.** `--egress-allow` in v0.9.0 could start
a box against a proxy nothing could reach, and on three of five hosts the pump never got the port
(`cannot bind 127.0.0.1:3128 in box: Address not available`). The pump now raises the box's loopback
itself and refuses to serve if it cannot, so readiness means reachable rather than bound. Reproduced
on an Arduino UNO Q and a Jetson Orin Nano with the shipped v0.9.0 aarch64 binary.
`scripts/acceptance-matrix.sh` exercises it, and says so instead of printing a tick on a host where
it cannot tell the fix from the defect.

### Fixed

- **The MCP server offered a language and then refused it.** The `run_code` schema advertised `sh`
  while a hand-written guard did not. The guard is the schema's list now, and a refusal names the
  accepted values.
- **`kern_execution_policy(cap_drop=("ALL",))` disabled the drop it asked for**, comparing a tuple
  against the string `"ALL"` and producing `drop_all_capabilities=False`. Only exact images convert;
  anything else raises and names the field to set.
- **kern's progress output no longer reaches a pipe.** Nineteen bare `eprintln!` lines now go through
  `progress!`, which prints only when stderr is a terminal. Errors, warnings and `kern: note:` still
  reach a pipe, because that is where they must arrive. `scripts/progress-is-tty-gated.py` keeps it
  true; converting the sites by hand found fourteen and missed five.
- **Three diagnostics reached `code_stderr` as though the workload had printed them**, lacking the
  `kern: ` prefix, plus five `kern compose:` lines whose prefix matched nothing.
- **A cgroup probe printed systemd's bus error onto kern's stderr.** `systemd-run ... -- true`
  inherited its stderr, so on a host with systemd installed but not booted the box's stderr carried
  `Failed to connect to bus`. Both streams are null now; the verdict was never in the output.
- **kern's diagnostics no longer land in a model's context.** `code_stderr`/`codeStderr` is stderr
  without kern's own lines, `runtime_notes`/`runtimeNotes` holds exactly what was removed, and
  `stderr` still holds every byte in order.
- **`kern_execution_policy` accepts `Sandbox`'s vocabulary**, `timeout_s` and `memory_mb` beside
  langchain's `command_timeout` and `memory_bytes`. Passing both halves of a pair is refused.
- **The OOM message never printed when kern runs as root or on a host with no systemd**, which is
  where the cap is most likely to be the only thing between a workload and the machine. The counter
  was read by walking kern's own cgroup ancestors, and on the direct-cap path the supervisor sat
  inside the cgroup `memory.oom.group` was about to kill. Verified on four hosts.
- **The registry recorded the supervisor's cgroup for every box**, and `kern stop` writes
  `cgroup.kill` into the path it records, so a stop would have killed the reporter.
- **A `-v` volume is mounted `nosuid`**, `/workspace` included. A `:ro` volume still fails hard.
- **`/dev/shm` reports the size the box actually has.** Unsized, `statvfs` reported half the host's
  RAM: a box held at 512 MiB told every workload it had 15.6 GB.
- **A warm interpreter could not import anything the image ships.** The driver ran `python3 -S`, so
  `import numpy` worked on a cold `run_code` and raised in a kernel cell. Dropped in both bindings.
- **`kern-sandbox` 0.1.36 on npm could not be installed**, its `package.json` depending on itself.
  Fixed in 0.1.37 and deprecated on npm.

### Changed

- **`deps_readonly` defaults to TRUE**, so a cell cannot change what the next cell imports. The route
  it closes is bytecode: a `.pyc` is validated on the source's timestamp and size, so a rewritten
  `.pyc` with the header re-pasted ran on the next import, invisible to `result.files`. A run-time
  write into `.deps` now gets `EROFS`; `deps_readonly=False` restores the old behaviour. It costs
  nothing at run time, the setup box compiling before the mount closes.
- **A timeout reports `exit_code = 137` in Python**, not `-9`, matching Node, the CLI and docker.
- **`integrations/pi` declares `engines: node >= 22`.** Measured: 20.18.1 fails at import, 22.11.0
  runs all 165 assertions.

### Added

- **`kern box --shm-size SIZE`**, for a workload needing `/dev/shm` sized differently from `--memory`.
- **`prewarm=N` in both bindings**: ~1.6 ms per call instead of ~37.8, measured over ssh, without
  giving up the fresh box, since a prewarmed box serves exactly one cell and is destroyed. A slot
  refills in ~70 ms, so N is a burst budget rather than throughput. Default `0` in the SDK, `1` in
  `kern-mcp`.
- **Every box gets a writable `/tmp`**, 64 MiB of tmpfs charged to the box's own memory cap. Nothing
  in it survives a call. `security_profile="untrusted"` gets none, deliberately.

**kern-sandbox 0.1.41** is documentation only, no code change from 0.1.40: the two package READMEs
moved their operational tail to `SANDBOX-NOTES.md` beside each binding. **0.1.40** answers an external
audit of the SDK, the pi extension and the LangChain integration.
