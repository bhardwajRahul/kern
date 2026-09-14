# Changelog

**CLI stability.** Since v0.7.0 the verbs, their flags and the `--json` shapes change incompatibly
only on a minor bump, never on a patch, and only after a deprecation entry here one release earlier.
`--json` is additive, so consumers must ignore unknown fields. A `cli_surface_is_frozen` test fails
the build on any undocumented change. Full detail for any entry is in the git history.

## Unreleased

Docker Compose, the sandbox SDKs and the MCP server. Each line is the change; how a defect was found
and why the fix is shaped that way is in the commit it came from (`git log v0.9.32..HEAD`).

### Docker Compose

- Each service gets its **own** network namespace on a bridge, which is Docker's arrangement. `--pod`
  keeps one shared namespace and one `127.0.0.1`, and is faster; `kern compose <file> config` prints
  which wiring a bring-up will use.
- **A box on a home or edge network had internet and could not resolve a name.** pasta copies the
  host's nameserver into the box, and on such a network that address is the LAN router, which is also
  the gateway pasta impersonates inside the namespace; pasta does not forward DNS queries unless it is
  told to, so they died there. MEASURED on a Raspberry Pi 5: `nc 1.1.1.1:443` connected and not one
  name resolved, silently. kern passes `--dns-forward` for a real (non-loopback) host resolver now. A
  laptop resolving through a `127.0.0.53` stub was never affected, which is why this was invisible
  until the board ran it.
- **A multi-service stack had no outbound on every host whose NIC is called `eth0`** (WSL2, most cloud
  VMs), while the same file on v0.9.32 reached the internet. kern renames the bridge veth to `eth0` for
  the workload's benefit, and pasta, told only `--config-net`, names its own tap after the HOST's
  interface: on those machines the two are the same name and pasta dies with `TUNSETIFF failed: Invalid
  argument`. kern names that interface itself now, and names it per namespace: `kern0` beside the veth
  on the bridge, and `eth0` where nothing is holding it (a pod's shared namespace). The second half is
  a fix of its own: that name used to be the host's, so a box saw `eth0` on a cloud VM and `wlp4s0` on
  a laptop, and anything looking for `eth0` inside a box worked on some machines and not others.
- Peers resolve by service name, by `networks.<net>.aliases` and by the name a service announces.
  `external: true` joins a network shared between projects, `network_mode: service:X` gets that
  service's namespace, and a bridge service has outbound and a resolver.
- Verbs: `run`, `cp`, `exec` (with the `--` Docker users type), `logs -f` over the whole stack,
  `ps -q`/`--services`/`--format json` (Docker's field names beside kern's, plus `Publishers`),
  `config`, and `systemd`.
- Flags: `-d`, `up --wait`/`--wait-timeout`, `--exit-code-from`, `--abort-on-container-exit`,
  `--no-deps`, `--build` (`--no-build` refused), `down -v`, `down --remove-orphans`, `--env-file`.
- Files: `docker-compose.override.yml`, `extends: {file: ...}`, `.env` interpolation, `env_file:` long
  form, anonymous volumes in long form, named volumes scoped to the project, secrets from an
  environment variable.
- Keys honoured: `cpu_shares`, `memswap_limit` (a total, unlike cgroup v2's field), `ulimit`,
  `runtime:`, `ipv4_address:`, `depends_on` conditions, an image's `HEALTHCHECK`/`STOPSIGNAL`/`Cmd`/
  `Entrypoint`/`Env`, a healthcheck in exec form, `--tmpfs uid=`/`gid=`.
- `network_mode: host` is applied, as Docker applies it, and now SAYS so: it removes the service's
  network isolation (the host's interfaces, loopback and sysctls become the service's), its peers stop
  resolving it by name, and any `ports:` it declares is a no-op. It was the one posture change in the
  file that happened in silence.
- Keys refused or named rather than ignored: an unknown service key (with the near-miss suggestion),
  `deploy.replicas`/`mode`/`placement`/`update_config`/`rollback_config`/`endpoint_mode`,
  `deploy.restart_policy`, and `deploy.resources.reservations` - a GPU request now says the service
  runs WITHOUT the device and where a device comes from.
- A dependency that never becomes healthy fails in a second naming the box, and the error now carries its
  own repair (`kern logs <box>`, and the three `healthcheck:` fields to look at). Without one it was
  getting the generic "a stack is a docker-compose.yml or a kern TOML" pointer: advice about writing the
  file, under a failure that had nothing to do with the file.
- Teardown stops dependents before dependencies and waits; an init that ignores the stop signal gets
  its grace; `down` stops a stack's NATs; a refused first service leaves no empty pod; a pod holder
  stops holding when its pod stops existing.
- A privileged port is moved once for the whole stack, before anything starts, and the message says
  what the move costs an ACME client. kern names the sysctl that keeps the port where the file wrote it.
- Inside a box: `localhost` resolves to something listening, `HOME` follows the user, a workload gets
  its image's groups, a command knows the box's name, a box name may be 200 characters, and a box that
  dies against its `pids` cap says so.
- **CLI, additive:** `--health-start-interval <sec>`, the flag behind Docker 25+'s `start_interval`.
  Nothing is renamed or removed; a box that does not pass it behaves exactly as before. `kern compose`
  sets it from `healthcheck.start_interval:`, which until now was read by nobody and dropped in
  silence.
- **A database that was ready in ten seconds reported `starting` for five minutes.** Docker 25+ splits
  the probe cadence in two: `Interval` for the steady state and `StartInterval` for the start period.
  kern read only the first, so an image declaring `Interval 300s, StartPeriod 300s, StartInterval 5s`
  (Immich's postgres) had its FIRST probe land 300 seconds in, and everything gated on
  `depends_on: condition: service_healthy` waited with it. Both are honoured now, from the image config
  and from a Dockerfile's `HEALTHCHECK --start-interval`, which kern used to accept with a printed
  apology for not applying it.
- **A failed `up` left its surviving boxes rebuilding forever.** `restart: always` is a promise about a
  workload, and a box that never started has none: it exits 125 (kern's box-not-started code, and
  Docker's), logs "never released: the launcher closed the pre-exec gate", and was restarted without a
  cap. Measured on a real stack: 26 rounds for one service, 15 for another. That case is budgeted now
  like `on-failure` is, it counts out loud (`never started (exit 125); retrying (3/10)`), and it says
  why it stopped. A workload that exits keeps Docker's uncapped contract, unchanged.
- **An explicit `docker.io/` prefix broke every pull**, and it is not an exotic spelling: it is
  Podman's recommended style and what Immich's official compose file ships. Docker Hub's API is at
  `registry-1.docker.io`, so kern was asking `docker.io` for a manifest and getting a document that is
  not one: unpinned it read as "no layers in manifest", pinned it read as a digest mismatch against a
  digest that does not exist in the repository. `docker.io` and `index.docker.io` resolve to the API
  host now, and `docker.io/alpine` means `library/alpine` exactly as bare `alpine` does.
- A relay test reported a phantom defect on a host that runs kern outside a delegated cgroup tree,
  which kern's own message calls "an ordinary ssh session on most distributions": the stack came up,
  the relay carried, and `exec` fail-closed because the command could not be put in the box's cgroup.
  The test threw away that stderr and the 126, so it could only say `Got: ""`. It reports both now,
  and the shared "this host cannot run the fixture" predicate knows that refusal, so such a host skips
  instead of going red.
- The compatibility rate ships with its corpus and its definition. The v0.9.32 claim of "14% to 94%"
  was the CEILING under a permissive definition; the strict one measures 35% on the same 259 files, and
  both numbers are in `docs/DOCKER-COMPAT.md` with the census scripts that produce them.

### Sandbox SDKs (`kern-sandbox`, Python and Node)

- A clean-code and security pass over the above closed four more holes: the session-reset and truncation
  notes were forgeable through the LangChain renderer (every frame is one list in the core now, and both
  surfaces recognise all of it), the LangChain shell policy built a `-v` that skipped the mount validator
  (workspace and `extra_box_args` both go through it), Node's `writeFile` opened its leaf by path after an
  `lstat` pre-check and now opens it through a pinned parent fd with the same `/proc/self/fd` backstop
  `readFile` had, and `restore()` re-implemented workspace containment instead of calling it. A workspace
  under a credential directory is also refused BEFORE it is created, rather than after.

- Published 0.2.0 through 0.2.18 on PyPI and npm. **0.2.0 was a MINOR bump because `fault.type`
  changes value for the same event**: an external `kern stop` was `oom` and is now `killed`, a workload
  that CHOOSES `exit 137` is no longer a fault, a crash is `fault=None` with `128+signal`, and a
  `KERN_BIN` that is not kern raises instead of reporting success.
- `fault` is read from kern's own descriptor, so a cell cannot forge a verdict from its output, and the
  taxonomy does not depend on the workload's language (measured on Node and Go, compiler included).
- A box that never started is `fault.type == "startup_failed"` returned by `run_code`/`run`, and RAISES
  from `kernel()`, where the box is the session rather than one call. Branch on `fault`, not on
  `exit_code`: a box that never ran exits 1 exactly like a script that did. **Every** error kern reports
  before the box exists is now one, decided from kern's own prefix at column 0 rather than from a list of
  message openings that had to be extended each time a caller met a new one: a missing `profiles=` name
  (`error: config:`) and an empty `image=` (`error: bad image reference:`) were both coming back
  `exit_code 1` with no fault. The verdict is refused whenever kern signalled that the box started or
  the box printed anything, so a workload writing kern's prefix cannot claim it never ran.
- A kernel a cell killed now names what ended it and what survived: "a prior cell ended it (oom). Files
  written to the workspace are still there; names and imports from the earlier cells are gone".
- Workspace I/O refuses what it cannot contain, and says which: an absolute path (it is never
  reinterpreted as workspace-relative), a `..` escape, a symlinked component at any depth (named as a
  symlink, not as `ELOOP`), a FIFO or device planted at the name, and a file over `max_bytes`.
- Mounts refuse the host's own sources (`/`, `/etc`, `/root`, `/boot`, `/proc`, `/sys`, `/dev`,
  `$HOME`, the docker socket), any path with a credential directory in it (`.ssh`, `.aws`, `.gnupg`,
  `.kube`, `.docker`, `.azure`, `.password-store`, `.netrc`, `.git-credentials`, `.pypirc`, `.npmrc`)
  and **kern's own state** (`$XDG_RUNTIME_DIR/kern`, the image cache, the config dir). There is no
  opt-out, deliberately: a job that needs one credential should be given that one file in the workspace.
- `setup=` is refused at construction unless it is a shell command string, and a `tmpfs` larger than
  `memory_mb` is refused because `df` would report space the cap will not allow.
- `egress_allow` is a route-level boundary: a raw socket to an IP is `ENETUNREACH` and DNS does not
  resolve, so only clients that speak to the HTTP proxy get out. A database does not.
- Both `SANDBOX-NOTES.md` pages carry what a box does that surprises people: the workspace is not
  capped, `df` and `nproc` report the host, a fault ends a `kernel()`, `network=True` shares the host's
  loopback, an image pinned by digest is reproducible and a tag is not.

- **On the binary `install.sh` serves today, the fallback sentence asserted something it could not know.**
  0.9.32 writes 2 of the 4 teardown bytes, so a resident `kernel()` cell that SEGFAULTED and one whose
  syscall the seccomp filter refused both came back `killed` with "an external kill (`kern stop`, a
  signal, or the host running out of memory)", which is false for both. The verdict cannot improve
  without the byte; the sentence now names the bound and says a newer kern separates the three. The
  taxonomy battery skips those two cases on a two-byte binary with the evidence, instead of failing an
  SDK that has nothing to decide from: measured, the released pair is 20 ok / 0 failed / 5 skipped and a
  current binary stays 27 / 0 / 0.

### MCP server (`kern-mcp`)

- **Every `run_code` reply names the kern that ran it** (`[exit 0 in kern 0.9.32]`), and the tool
  description leads with what a client's own shell cannot do (seccomp, no capabilities, read-only root,
  its own `/dev`, no host filesystem) instead of with "on the user's own machine". A reviewer wired the
  server into Cursor correctly and the agent answered "the sandbox run completed successfully" from its
  own python, never calling the tool: the description had described the client's terminal too, and
  nothing in a reply could contradict a run that never happened.
- An argument a tool does not have is refused rather than dropped: every schema is
  `additionalProperties: false` and the server checks what it was sent against what it advertised, so a
  call carrying `image` or `network` gets `-32602` naming it, and nothing runs. It used to run the code
  under the server's own posture and answer `[exit 0]`, which tells a model its request succeeded.
- Box output is untrusted text on its way into a model: terminal escapes and control bytes are
  stripped, and **both** surfaces' framing is neutralised, this server's (`[exit N]`, `[stderr]`,
  `[rich result]`, the truncation and session-reset notes) and the LangChain renderer's
  (`[sandbox: ...]`), so a forged marker reads `[printed by the code, not the sandbox: ...]`.
- The `run_code` description states the image and exactly one contract about state: a fresh box per
  call, or one warm interpreter under `KERN_MCP_KERNEL=1`. In kernel mode a fault ends the interpreter,
  and the reply for the cell that died says so once.
- `docs/MCP.md` documents what a client can and cannot reach, measured: the server's environment does
  not enter the box and there is no knob to pass it in, there is no mount knob, calls are serialised,
  two clients get two workspaces, `KERN_MCP_QUIET` never hides a verdict, and `KERN_MCP_SETUP` is paid
  once.

### `kern-pi` (the pi coding agent's extension)

- **1.0.1 on npm.** 1.0.0 asked for `kern-sandbox: ^0.1.41`, and for a zero-major version a caret range
  stops at the next MINOR, so it resolved 0.1.x and never 0.2.x: a Pi user's model was reading the fault
  verdicts from before this year's chain, because the extension writes `[kern: <fault.type>]` into the
  stream the agent reads. The range is `^0.2.12` now. Verified by installing it: `npm i kern-pi` pulls
  0.2.12, the package loads, and its file tools refuse `/etc/passwd`, a symlink planted in the workspace
  (named as a symlink) and a relative path from the agent, while a legitimate write lands.
- The README gives the npm route first and the clone second.

### Runtime and CLI

- `--memory 64` is 64 BYTES and the message says so instead of sending the reader in a circle.
- `kern ps`/`top` size the NAME column from the data; `ps --json` reports paused and orphaned boxes;
  the Docker-shaped NDJSON no longer calls a paused container `running`; the orphan warning names the
  directory it looked in and both causes before it suggests anything destructive.
- Every verb the parser accepts appears in `--help`, with aliases in the description column.
- A FIFO as a volume source is refused instead of hanging the box forever; a refused `-v` says what is
  under the source; `kern box --ip <addr>`; a pod bridge is validated against the host's routes and the
  loopback range; pods work on a host with no systemd and no elogind.

### Tests, gates and examples

- New batteries and gates, all in CI: `fault-taxonomy-battery.py` (27 cases), `docker-vocabulary.py`,
  `md-links.py`, `launch-dryrun.py`, `e2e-semantic.py`, `build-corpus-census.py`,
  `declared-bind-census.py`, and a `loopback-census.py` whose zero means something.
- 1330 Rust, 512 Python and 110 Node tests, and the count is gated against the README.
- `examples/` moved from 103 flat files into eight directories, nothing deleted, with one example that
  starts from a `docker-compose.yml` rather than from kern's own TOML.


## v0.9.32 - 2026-09-09

**A published port now binds `0.0.0.0`, not `127.0.0.1`. Read this one.** `-p 8080:80` and a compose
`ports: "8080:80"` bind every interface, which is what Docker does and what a file written for Docker
means. Until now kern bound loopback and warned, so a stack that looked published was reachable only
from the host. `[kern] publish_bind` in `kern.toml` is a ceiling no file can widen, and an explicit
`127.0.0.1:8080:80` still means loopback.

**Docker Compose compatibility went from 14% to 94%**, measured before and after on the same neutral
corpus of 259 files, one per repository, sampled across 733 repositories: the share of files kern
runs with no behavioural difference from what the file says. [CORRECTED: see "Corrected" under
Unreleased. 94% is the ceiling, not this definition, which measures 35% on the same corpus.] What remains is dominated by keys asking
kern to be less confining than it is (`privileged: true`, `security_opt`) and by `network_mode: host`,
which one namespace per stack cannot express. The earlier "15% irreducible" was an artefact of a
corpus weighted toward those keys.

**Twelve compose keys stopped being warnings and became behaviour**, among them `mem_reservation`
(cgroup `memory.low`), `devices:`, `dns:`, `logging:`, `tty:`, `stdin_open:`, `secrets:` long syntax,
and an image's own `HEALTHCHECK` and `STOPSIGNAL`. A string `command:` is now an argv rather than a
shell line, a tagged block scalar folds, and an empty named volume is seeded from the image as Docker
does.

**`networks:` is a boundary, not a warning.** Under `--no-pod`, services with no network in common
cannot reach each other by name or by address, and `internal: true` is the absence of NAT rather than
a filter, so a published port does not open a way out. In a pod the two say something different, and
both are stated at bring-up. A key that is absent means the `default` network, which is 52 of 187
files rather than the 18 that name one.

**`${VAR:?message}` refuses the file instead of substituting an empty string.** A stack whose
password variable was unset started with an empty one.

**An image's file ownership survives the unpack**, so a service running as a non-root user can write
the directories its image gave it. A named volume inherits the image directory's owner and mode, not
only its contents, and `kern rmi` no longer reports a removal it did not perform.

**A service secret is written with the mode the Compose Specification mandates.** It was `0400` in a
`0700` directory, so no image running as a non-root user could read its own secret. `target:`, `uid:`
and `gid:` were read and dropped in silence; they are applied or named.

**`kern run` no longer pays for a systemd scope it does not need: 4.70 ms to 0.87 ms.** It bought its
caps with a transient `systemd-run --user --scope`, one per invocation; it now caps directly under
kern's delegated `kern.slice`, the way `kern box` already did.

```
                  median     p99      max
before             4.700    5.735   15.304 ms
after              0.870    1.267    1.487
```

The tail moved more than the median because a D-Bus round trip to a shared user manager is a queue.
Throughput at concurrency 200 goes from 86 to 4052 runs per second: a path that serialises on a
shared service gets worse as concurrency rises, and a benchmark at concurrency 1 reports that only as
"slow". Finding the delegated slice turned out not to be the same as being allowed to enter it:
cgroup v2 delegation containment needs write access to the `cgroup.procs` of the common ancestor, and
a host outside that tree gets the scope path rather than an uncapped run.

**`kern exec` stopped refusing where there was no cap to escape**, and its fail-closed refusal now
names both causes and the way through (`KERN_ALLOW_UNCAPPED=1`). A `--health-cmd` probe is never
refused. `kern exec` and every health probe now run with the image's environment rather than a bare
one, and `kern stop` sends the stop signal once instead of twice.

**`kern doctor` names the cgroup it probed** on every row that denies a cap, so the verdict can be
checked against `/proc/<pid>/cgroup` instead of taken on trust, and it asks about both directories a
box can be capped in. `kern inspect --json` gains `memory_max_enforced`, read back from the box's own
cgroup: `memory_max` is the value the box was started with, and on a host that caps another way the
two differ.

**A box's terminal has a name.** `tty` inside an alpine box printed "not a tty" while `isatty` said
otherwise, because the `-it` pair was allocated on the host and the box's private devpts does not
contain it. It is now allocated from the box's own devpts and the master passed back over a
socketpair, so both C libraries resolve it. Certified on Fedora 44, CentOS Stream 10, Rocky Linux
10.2, Debian 13, openSUSE Leap 15.6 and Ubuntu 24.04, three with SELinux Enforcing.

**CLI surface: six flags added, none changed or removed.** `--dns`, `--dns-search`, `--dns-option`,
`--log-max-size`, `--log-max-file`, `--secret-mode`. Additive, so nothing that runs today stops
running.

## v0.9.31 - 2026-09-09

**If you use `kern exec`, this release is the one that makes it obey the box's limits.** It did not.
A command run through `kern exec` was placed in the CALLER's cgroup, outside the box's `--memory` and
`--pids-limit`, and said nothing about it. Measured from the host by pid, with the box's own PID 1 as
the control and the exec'd process verified to be in the box's PID namespace:

```
box PID 1                  .../kern.slice/kern-box-<tag>-<pid>     capped
the kern exec'd process    .../app.slice/app-<the caller>.scope    the CALLER's cgroup
```

A fork bomb or a memory hog started with `kern exec` therefore ran without the ceiling the box was
given. Namespaces and seccomp always held; it is the resource cap that leaked. The placement now
happens BEFORE the namespaces are joined, which is the only order in which the kernel permits it:
afterwards the box's own cgroup is the root of its namespace and the common ancestor cannot be named,
so both `clone3(CLONE_INTO_CGROUP)` and a write to `cgroup.procs` answer ENOENT.

**The cost is real and is stated rather than hidden.** Placing before the `setns` means a
`cgroup.procs` write, which takes an RCU grace period: `kern exec` is back to 11.7-25.8 ms on a quiet
host against 1.7-2.2 without it. The faster path shipped in v0.9.3 was faster because it was not
applying the cap. `clone3(CLONE_INTO_CGROUP)` is still used where it is correct, on the box START
path, which places its child before entering any namespace.

**A command killed by the box's memory cap now says so.** `memory.oom.group` kills the whole box, the
exec'd command included, and it goes by SIGKILL, so the process that would explain it is the one being
killed. Before, the caller saw exit `-9` with empty stdout and empty stderr. A reporter now waits
outside the group and names the cause. The exit code is still `-9`: that part belongs to the kernel.

**A box whose registry record is lost stays visible.** kern's registry lives in
`$XDG_RUNTIME_DIR/kern/instances`, and `/run/user` is swept by `systemd-tmpfiles`, cleared on logout,
and deleted by anyone who reads it as scratch. The box does not care: it keeps running. Before, only
kern forgot, completely - the box vanished from `ps`, `kern stop <name>` answered "no running box",
and nothing could reach it again. `ps` now reads what the kernel still holds and names those boxes
with their supervisor pid on stderr, on hosts with a delegated cgroup and, through `/proc`, on hosts
without one. It does not invent a table row for them: there is no record, so there is no uptime, no
ports and no health to show.

**Multi-stage builds produce an image that runs.** `FROM <stage>` printed `built` and left an image
that failed at `kern box` with "no layers in manifest", because the final image rested on a stage's
overlay chain. The final image is now materialized, fail-closed. `COPY` also stopped flattening
directory modes: a rootfs shipping a 1777 `/tmp` or a 2755 setgid directory came out 0755, and the
program that needed it failed for a reason nothing in the Dockerfile explained.

**Compose reads files it used to refuse, and refuses files it used to accept in silence.** A
`command:` continuation line starting with `-` was read as a sequence entry, so `--source`, `-drive`
and `-netdev` broke a plain folded scalar; two real files from public repositories now parse. `!!str`
is accepted over a scalar and refused over a list or a map, where it used to be dropped in silence
and the box started with something else. `tmpfs:` has ONE grammar again: `kern box --tmpfs
/run:size=64m` used to fail on the exact spelling `kern compose` produced, while `/run:rw` and
`/run:exec` - both valid Docker - were read as sizes and refused. An IPv6 port refusal now names the
missing feature instead of suggesting a typo, and `/dev/shm` and `/dev/pts` say the mount is already
there rather than giving the generic refusal.

**`kern build prune` refuses arguments it used to ignore.** `kern build prune 0` was read as "keep
nothing", ran with the default 20, and reported "kept the 20 newest".

**Fixed, no interface change:** a memoised runtime path outlived the directory it named, so anything
that cleared `/run/user` left every later registry write in that process failing, for the life of the
process; the layer cache treated a sentinel without its directory as a hit, and the build then died
on a `mount(overlay)` ENOENT that named neither.

**Known and unchanged:** the resource caps are verified on one machine. CI does not start boxes, and
the second reviewer's host has no cgroup delegation, so `--memory`/`--pids-limit` enforcement has one
witness. The squash that `FROM <stage>` and `push` share loses hard links and fills sparse files.
Compose networks do not isolate services from each other: one stack is one namespace, and `up` says so.

## v0.9.3 - 2026-09-07

**If you run kern on Ubuntu 23.10 or later, your install needs one action.** That is not a new
feature, it is the answer to "why does no box start", and it is here rather than under new
capabilities because it is the entry a reader scanning for "does this release affect me" needs to
find. Those releases ship `kernel.apparmor_restrict_unprivileged_userns=1`, which permits the
namespace and refuses the rootless uid map, so nothing starts. kern now ships the profile:

```
kern doctor --apparmor-profile | sudo tee /etc/apparmor.d/kern >/dev/null
sudo apparmor_parser -r /etc/apparmor.d/kern
kern doctor
```

**CLI, additive:** `kern doctor --apparmor-profile` writes that profile to stdout and exits without
running any check. It exists because the install line has to be runnable by the person reading it,
and a repo-relative `packaging/apparmor/kern` is not: the release tarball carries the binary alone
and `cargo install` copies one file, so most readers have no `packaging/` directory. The binary
carries the profile instead. Nothing is removed or renamed.

**What that file does NOT do**, because installing one into `/etc/apparmor.d/` on a program's
say-so deserves the sentence: it grants exactly one permission, `userns`, and confines kern in no
way at all. Not its paths, not its capabilities, not its syscalls. Removing it returns the machine
to its previous state. It is deliberately not a confining profile: kern's job is to confine the
workload, and a second weaker mechanism aimed at kern itself would mostly invite the belief that it
was doing something.

The third command is not politeness. AppArmor attaches at `execve`, so a kern already running when
you load the profile does not pick it up. Measured on Ubuntu 24.04 with the restriction left at 1:
without the profile no box starts, with it `kern box`, `kern pod create` and the full acceptance
matrix all pass, and the sysctl is never touched.

`kern doctor` prints that install line, names the path it is running from because AppArmor attaches
by path, and offers `sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` only after it, with
its cost: that one lifts the restriction for every program on the machine and is lost at reboot.

**`kern doctor` said "ready" on a stock Ubuntu 24.04, where no box can start.** Its userns probe
called `unshare(CLONE_NEWUSER)` and stopped there. Ubuntu 23.10 and later ship
`kernel.apparmor_restrict_unprivileged_userns=1`, which PERMITS the namespace and refuses the
rootless uid map, so the probe succeeded on a host where the very command doctor then suggested,
`kern box hello`, failed. Ubuntu is the most common distribution kern is installed on and that was
its default state.

The probe now runs the sequence a box actually runs, in the same order: unshare, deny setgroups,
write the uid map. Measured on a stock cloud image, both directions:

```
default                       ✘ the namespace is allowed and its uid map is REFUSED - no box can start
                              not ready - 1 blocker(s)
apparmor_restrict...userns=0  ✔ enabled
                              ready - `kern box` will run here
```

The AppArmor line no longer hedges with "if boxes fail with EPERM". It reports what the knob is set
to and leaves the verdict to the check that measured it, so a host carrying the restriction with a
profile for the kern binary is told it is fine rather than warned at.

**On a host whose SELinux policy refuses pasta's netns watch, the pod's pasta no longer exits by
itself.** kern retries with `--no-netns-quit` there ([#6](https://github.com/getkern/kern/issues/6)),
and a pasta started without the watch does not notice the namespace disappear, so `kern pod rm` and
`compose down` are what stop it rather than pasta stopping itself. Nothing to do differently; it
matters if you run a mixed fleet, because the hosts that take the retry and the hosts that do not
now have two different pasta lifecycles, and only the first depends on teardown running.

**`stdin_open:` and `tty:` in a docker-compose.yml no longer produce an alarm.** They used to warn
"ignored (unsupported)" matched on the KEY rather than the value, so `tty: false` warned about
nothing and a working stack was told a feature was missing
([#7](https://github.com/getkern/kern/issues/7)). A compose service is always detached, so `tty:`
has nothing to act on and is silent; `kern exec -it <service>` gives a real PTY in the running box
when one is wanted. `stdin_open: true` still warns, because it is a real difference from Docker: the
service's stdin is at EOF rather than held open, so a program that blocks on it exits at once.

**The Rust test suite had never been built for aarch64, though the binary always was.** So every
"the tests pass" statement this project has made was an x86_64 statement, silently, for as long as
ARM has been a supported target. Two lines caused it, both in test code and both invisible on
x86_64-gnu: `pthread_t` is a `c_ulong` on glibc and a `*mut c_void` on musl, and the pointer form is
not `Send`; and `ioctl`'s request parameter is a `c_ulong` on glibc and a `c_int` on musl. It now
builds and runs on ARM: **577/577 on a Raspberry Pi 5 (kernel 6.6) and on a Jetson (5.15-tegra)**,
on the hardware rather than under emulation. Under `qemu-user` one `flock` contention test fails
reproducibly and passes six times out of six on the boards, so that red is an emulation artifact and
not a name-collision bug on ARM.

That is the largest instrument defect in this cycle: not a probe reading the wrong thing, but a
whole suite that was never executed on a target kern ships for.

**A refused netns watch is only fatal in newer passt, and where it is not there is nothing to fix.**
Read out of three installed binaries rather than inferred:

| passt | ships in | on a refused watch |
|---|---|---|
| `0.0~git20230309` | Debian 12 | `inotify_init(): won't quit once netns is gone`, and it keeps the NAT |
| `0.0~git20240220` | Ubuntu 24.04 | `netns dir open: %s, exiting` |
| `0^20250919` | Fedora 43 | `netns dir open: %s, exiting` |

So a Raspberry Pi on Debian 12 needs no retry and never could have: issue #6 cannot occur against
the tolerant build. The condition became fatal between March 2023 and February 2024, which is
exactly the range the retry covers.

**Fixed: `--memory` and `--pids-limit` reported "accepted but NOT enforced here" over a box capped
exactly as asked.** Reported on WSL2 and reproduced on a Raspberry Pi 5 and a Jetson Orin Nano,
where `--memory 256m --pids-limit 64` printed both notices while the box's cgroup held
`memory.max=268435456` and `pids.max=64`. The check was right and was asked in the wrong place. It
read `/proc/self/cgroup`, and on the systemd-scope tier that is the SUPERVISOR, which kern parks in
a sibling leaf so a whole-box OOM cannot take it with the workload. From there the walk goes to the
ancestors and never reaches the box's leaf, which is a sibling rather than a parent; the only
ancestor carrying a memory ceiling is the scope, deliberately set to the request plus kern's
supervisor headroom, so "capped at or below the request" was false by design. Both notices were
wrong on the whole of that tier, and on the direct tier the check never runs at all, so it had never
once fired correctly.

The same mistake had a second instance, found by adding one diagnostic line to the reproduction
script rather than by reading the code. The `KERN_NO_SCOPE` opt-out warned from a point BEFORE the
box exists, where whether the cap will bind is not yet knowable, on the belief that the opt-out
skips the box's own cgroup as well as the scope. It does not. On x86_64 that printed "accepted but
NOT enforced here" while the box held `memory.max=268435456` and a 400 MB load was killed with exit
137. The warning now comes from one place on every box path, after the caps are written, against the
box's own cgroup. The Raspberry Pi finding the opt-out warning was written for is unchanged and
still reported: measured on a Pi 5 and a Jetson, the opt-out leaves `memory.max` and `pids.max` at
`max` and a 400 MB load survives, and kern says so. The notice now follows the cgroup rather than
the code path.

The enforcement byte on `KERN_STARTED_FD` was already correct: it takes the box's directory
explicitly, for this exact reason. So an SDK reading the byte saw "enforced" while a human reading
stderr saw the opposite, in the same run. Both now read one binding, so they cannot disagree. If you
scripted around the false notice, remove the workaround; if you concluded your caps were not
working, they were, and `--memory 256m` was killing at 256 MiB throughout.

**Fixed: a box refused for running out of process slots was told to check user namespaces.** A
reviewer hit it with a tightened `ulimit -u`:

```
error: sandbox: fork(idmap helper) failed: Resource temporarily unavailable (os error 11)
hint: needs unprivileged user namespaces and a valid --rootfs directory
```

The message is exact and the hint names two things that are both already fine, because the code
could not have reached that fork otherwise. `EAGAIN` on a fork is a process-limit problem, and
`RLIMIT_NPROC` is per-UID and counted across the whole system, so another program owned by the same
user can exhaust it, and it counts TASKS rather than processes. That last clause is not a detail:
the reviewer who reported the hint then compared `ulimit -u` against a process count, got 10 against
149, and concluded the kernel was accounting something unobservable. Measured here, an x86_64 desktop
owned 208 processes and 1918 tasks and the limit at which a single fork began to succeed was 1932, so
against the task count the threshold IS the count. The hint now names `ulimit -u`, the task count and
`LimitNPROC=`. Every other setup failure keeps the hint it had. Same shape as the pull hints, which branch on the message
rather than on the variant for exactly this reason.

**`kern --version` now says which build it is.** It answered `0.0.0` for every binary not cut by the
release workflow, which is every binary anyone compiles from source, so two builds of the same tree
were indistinguishable. That is not hypothetical: during the work above, a binary built ten minutes
before the fix was compared against one built after and reported as if it were the same program. A
reviewer made the same point from the other side, noting that a test script had to print a
`sha256sum` to tell two builds apart, and that the workaround existed only because the binary could
not answer.

The version is still the tag and nothing is carved into the source. A release binary prints the tag
exactly as before (`kern 0.9.3`), because the workflow stamps `Cargo.toml` and that value passes
through untouched. A build from source prints `git describe` instead
(`kern v0.9.2-45-gf7622ee-dirty`): the nearest tag, the distance from it, the commit, and whether the
tree was dirty. Where git cannot answer, a source tarball or a vendored build, it falls back to
`0.0.0`, which is today's behaviour, so nothing regresses when the information is unavailable.

## v0.9.2 - 2026-09-06

**Cut for a defect the first person to try compose would hit.** A `docker-compose.yml` with ONE
service came up with no network at all, for the whole 0.9 line. The auto-pod was gated on two
services or more, on the reasoning that a pod's other job is letting services find each other and one
service has nobody to find; but the pod is also the only thing that attaches `pasta`, so a lone
service got no pod, no NAT and no `/etc/resolv.conf`.

It does not present as a missing network, which is why it survived: the image ships its own
`resolv.conf` and it looks healthy, so the failure surfaces as `Could not resolve host` and every
diagnosis goes after DNS. `curl http://1.1.1.1` from inside the box failed in **0 ms**. There was no
route. Reported from a Mac running Lima with a Fedora guest, but the platform was never the variable:
reproduced on x86_64 Linux with pasta installed, changing only the service count.

**Every compose test in this repo ran three services, which is how it shipped.** The single
`services:` in the Rust suite points at an unreachable registry and never starts a box, so the
one-service path had no coverage anywhere. `scripts/acceptance-matrix.sh` now has a case for it, with
its two new assertions exercised in `--self-check` including a negative control on the pre-fix
summary line. The case goes RED on the v0.9.1 binary and green here, confirmed by an external
reviewer on their own host rather than only here.

### Fixed

- **A one-service compose stack had no egress.** The auto-pod condition tested a service COUNT while
  the property it stood in for was "does this stack need a managed network". It now creates a pod
  whenever any service is not on the host net, which is what the comment above it always said it did.
- **`kern pod ls` and `pod ls --json` reported double the members.** They counted lines in the pod's
  shared `hosts` file, and a compose member writes two of them (the qualified `<pod>-<service>` and
  the bare alias) while a `kern box --pod` member writes one. Measured on the shipped v0.9.1: 1, 2 and
  3 services read 2, 4 and 6, while `kern ps` read 1, 2 and 3. Both now read the registry `kern ps`
  reads, scanned once rather than per pod. The two views had been unified so they could not disagree;
  they could not, and both were wrong, while a third reader had the right answer.
- **`compose up` never said whether the stack had egress**, only that services could reach each other,
  so a stack with internet and one without printed the same sentence. On a reused pod that line is the
  only one printed. It now names the state, and `docs/DOCKER-COMPAT.md` lists all five instead of two.
- **`restart:` in a pod does not survive a reboot, and now says so.** A pod member is supervised
  in-process because a systemd unit that outlives the pod holder cannot re-join its namespace. The gap
  predates this release and reached only multi-service stacks; the auto-pod now reaches one-service
  stacks, so `up` prints a note instead of trading reboot-survival in silence.
- **`has_outbound` answered from `resolv.conf` alone.** Kill `pasta` while the holder lives and the
  file stays on disk, so the predicate reported egress for a pod with no route. It now also requires a
  live pasta, verified by `comm` because passt re-execs into an ISA variant and a pid can be reused.
  Found by an external reviewer reading the diff, not by a test here.
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
