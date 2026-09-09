# Changelog

**CLI stability.** Since v0.7.0 the verbs, their flags and the `--json` shapes change incompatibly
only on a minor bump, never on a patch, and only after a deprecation entry here one release earlier.
`--json` is additive, so consumers must ignore unknown fields. A `cli_surface_is_frozen` test fails
the build on any undocumented change. Full detail for any entry is in the git history.

## Unreleased

**Five silent differences from Docker were named, and one of them was a refusal.** The compatibility
rate for `kern compose` had been measured from kern's own warnings, which makes it blind by
construction to any difference kern does not know it has: a file kern says nothing about counts as
perfect. Five such differences are now stated at `up` and at `config`, and the measured rate fell
when they were, because the differences were always there and the measurement was not.

A service with no `mem_limit:` runs under kern's 512 MiB ceiling while a Docker container with none
runs uncapped, so a service that needs more is OOM-killed at a number written nowhere in the file.
243 of the 259 files in the neutral corpus have at least one service in that position; kern names
them once per stack, quoting the constant the cgroup actually enforces rather than a copy of it.

In one shared namespace the services share `127.0.0.1`, so a port a service binds on the loopback is
reachable from every other service in the stack. That is an exposure rather than a failure, which is
why nothing had ever reported it, and it now gets a line on any multi-service pod. A single-service
stack is told nothing: there is no peer to be reached from.

A service mounting `/var/run/docker.sock` (or `/run/docker.sock`) is told that kern is daemonless and
that there is nothing behind the path. That difference cannot be closed, and naming it is the whole
remedy available.

`RUN --mount=type=secret` and `--mount=type=ssh` are now REFUSED instead of silently stripped. The
command was written because the credential would be there, so dropping the flag runs it
unauthenticated: either a 401 whose message points at the registry rather than at the discarded flag,
or a build that succeeds against a public mirror and ships something other than what was asked for.
`type=cache` and `type=bind` stay dropped, since those cost a rebuild rather than a wrong answer.

`external: true` on a volume that does not exist is refused, as Docker refuses it. The top-level
`volumes:` block was previously not read at all, so kern auto-created the volume and the service
started on empty storage where somebody else's data was supposed to be. An `external:` declaration
carrying `name:` is honoured, and that name is validated as a volume name: a first version let it
name a host path, which kern's `-v` classifier then bind-mounted.

**`${VAR:?message}` refuses the file instead of substituting an empty string.** That form exists to
stop a file being rendered without the value, and it is what a compose file writes for a password or
a token: a stack came up with `MYSQL_PASSWORD=`. Every variable with no value is named, not just the
first one found.

**`kern stop` sent the stop signal TWICE, so a workload's shutdown handler ran twice.** `stop`
signals every box in phase 1 so a stack tears down in parallel, and the per-box wait then sent the
same signal again. A shell trap is re-entered on the second delivery: measured on a handler that
takes 2 s, `stop` took 4006, 4007 and 4007 ms across three runs with the box's own log showing the
trap entered twice, against 2005/2007/2006 ms and a single entry after the fix. Latency is the
smaller half of it - a handler that flushes, deregisters or writes a final record did all of it
twice, on every stop. The other arms are unchanged and stay immediate: a workload that ignores the
signal is killed at once (4 ms), one that does not touch it dies at once (5 ms), and a declared
`stop_grace_period` is still honoured to the millisecond.

**`kern exec` and every health probe ran with a bare environment inside an image whose workload is
not root.** `exec_in_box` inherits the box's environment by reading `/proc/<pid1>/environ`, and that
read fails there: the file belongs to the workload's mapped uid and reading it needs `CAP_SYS_PTRACE`
in the box's user namespace, which the box has dropped. Measured on
`rabbitmq:3.12-management-alpine`: the environ is owned by uid 100099 and unreadable, so `kern exec`
saw the default `PATH`, `rabbitmq-diagnostics` was not on it, and the image's own `HEALTHCHECK`
reported the service UNHEALTHY while its management API answered 200. kern now records the
environment it gave each box, in a NUL-separated sidecar beside the health record, and `exec` and the
probe read it back; an explicit `-e` still wins. The same stack now reports `healthy`.

**An image's file OWNERSHIP survives the unpack, so a service that runs as a non-root user can write
its own directories.** Layers were extracted with `--no-same-owner`, which gave every file to the
caller: inside a box that is uid 0, so an image that `chown`s a directory to a non-root user and then
runs as that user could not write to it. Measured on three real stacks, all of which died on it:
Prometheus (`mkdir data/: permission denied`), Kibana (`EACCES` on its uuid file) and Logstash. All
three now come up, and Kibana answers 200.

The box already maps a full subordinate range, so the ownership those images want was always
representable on disk; it was lost because `tar` ran outside the namespace where the range exists.
The unpack now runs as root of a namespace carrying that same range, which also let the hand-rolled
single-uid map inside the OCI crate be deleted in favour of the one owner of that primitive. An image
that names a uid the range cannot cover (OpenShift-style ids in the millions) is retried the old way,
because `tar` exits 2 on such a chown and the image must stay pullable; measured on a crafted layer.

Directory ownership needed a second fix: the layer merge RE-CREATES directories and only moved the
files, so on `kibana:7.16.1` all 26920 files carried their ids and all 6680 directories did not. The
merge now restores owner and mode on directories and symlinks, for the reason the code already gave
for restoring the mode.

**A named volume inherits the image directory's owner and mode, not just its contents.** `cp` fills a
directory and leaves the directory alone, so a volume mounted where the image put an EMPTY directory
owned by a non-root user copied nothing and stayed owned by in-box root. That is the Prometheus case
exactly, and it survived the ownership fix above until the volume root was given the same treatment.

**`kern rmi` no longer reports a removal it did not perform.** With ownership preserved, an image
leaves directories this process does not own, and unlinking inside one needs write permission on it:
measured, `kern rmi kibana:7.16.1` printed "freed 1.1G" and left 85 entries behind. The removal now
retries as root of the mapped namespace, the result is returned instead of discarded, and the freed
figure is reduced by whatever survived. `kern volume rm` takes the same path, for the same reason.

**A string `command:` is an argv, not a shell line.** kern wrapped it as `sh -c "<string>"`, so
Docker's own `awesome-compose` WordPress sample - `command: '--default-authentication-plugin=…'` on
`mariadb` - started `sh` with that string as an OPTION and the database died on every start with
`sh: 0: Illegal option --`. The Compose Specification is explicit that the shell-form syntax "does
not implicitly run in the context of the SHELL instruction" and tells the author to write
`/bin/sh -c` when they want one. Measured on the neutral corpus: of 83 string commands, 13 use shell
syntax and all but two of those already write their own `sh -c`, which splitting preserves verbatim.
Two more files carried the same broken shape (a command beginning with `-`).

A string `entrypoint:` no longer DROPS `command`. That rule belongs to Dockerfiles, where
`ENTRYPOINT some string` becomes `/bin/sh -c "…"` and has nowhere to put arguments; the specification
says the Compose string form does not run in a shell, so the premise is absent. kern cleared the
command and warned, silently discarding arguments the file asked for.

**A block scalar carrying a tag was not folded at all.** `command: !!str |` with a body left the
literal `|` in the value, so the service tried to execute a program called `|`. The fold scan
required the value to BEGIN with the indicator and did not look past a tag it otherwise accepts. It
was invisible while a string command was wrapped in a shell, where it became a run-time syntax error
instead of a start failure, and the test that covered it asserted only that some argument contained
the body.

**An empty named volume is seeded from the image, as Docker does.** Docker copies the image's content
at the mount point into a named volume the first time it is used while still empty; kern mounted an
empty directory over the top, so a service found nothing where its image had put a default
configuration or an initial database, and then failed with an error of its own making. Measured on
`nginx:alpine` with an empty volume at `/etc/nginx`: 0 files before, 8 after. Only when the volume is
EMPTY, and only for a NAMED volume; a bind mount of a host path is never touched. A multi-layer image
is read through the kernel-merged overlay view, so a file a higher layer deleted does not come back.
`merged_view_extract` gained an `Extract` enum for this: a first version placed the directory itself
inside the volume, one level too deep, and `Option<&str>` could not say which of the two was meant.

**The image's own `HEALTHCHECK` and `STOPSIGNAL` are read.** Docker runs an image's check whether or
not the compose file mentions one, and `depends_on: {condition: service_healthy}` waits on exactly
that; kern read neither, so such a service reported `HEALTH = "-"` forever. Measured on a box whose
image declares a check: `healthy` when it passes, `unhealthy` when it fails, `-` on `main` in both
cases. A `healthcheck:` in the file replaces the image's entirely, numbers included, which is
Compose's rule. `--stop-signal` became optional at the flag boundary so an explicit `SIGTERM` can be
told apart from the default and still win over an image that asks for something else.

**A double close in the log-rotation test was corrupting unrelated tests.** `CappedLog` closes its
descriptor in `Drop`, and the test added with `logging:` closed it by hand as well. The second close
succeeds, having destroyed whatever the operating system handed that number to in the meantime, so
the suite failed intermittently in a different unrelated test each run with `remove_dir_all`
panicking `closedir: Bad file descriptor`. Measured at 3 failures in 14 runs before, 0 in 14 after,
0 in 11 on `main`. A source-scan test now fails the build if any holder closes that field again.

**Six Docker Compose keys stopped being warnings and started being behaviour.** Each was chosen from
what real files ask for: 240 compose files from 221 public repositories were parsed and their values
counted, so the work follows the distribution rather than Docker's vocabulary.

`devices:` is applied. It is a bind mount, which is what it always was: measured before any of this,
`kern box -v /dev/kvm:/dev/kvm` already gave a workload a working `crw-rw---- 10, 232 /dev/kvm`, so
refusing the key whose only purpose is to say that was withholding a spelling, not a privilege. The
node arrives with the host's own owner and mode, so a caller who cannot open it on the host cannot
open it in the box. `/dev/net/tun`, which is 37 of the 83 `devices:` values in that corpus, maps to
`--tun` instead of a plain bind, because the node alone is useless without the `CAP_NET_ADMIN` that
creating a tunnel interface needs. Docker's third field is honoured where kern has it: a spec with no
`w` becomes a read-only bind.

`dns:`, `dns_search:` and `dns_opt:` are applied, through three new flags: `--dns`, `--dns-search`
and `--dns-option`. A box that names none is byte-identical to every box kern has started so far:
the image's own `/etc/resolv.conf` is left alone, including the empty one the debian family ships. A
`--dns` that is not an IP literal is refused at the flag, because glibc silently skips a `nameserver`
line it cannot parse and the box would otherwise run with no DNS and no message.

`logging:`'s `max-size` and `max-file` are applied, through `--log-max-size` and `--log-max-file`.
kern's capture has always been a size-capped rotating log; the options now set its cap and its
generation count, with Docker's counting (`max-file: 3` means the active file plus `.1` and `.2`).

`links:` is applied. The alias lands in the source service's `/etc/hosts` in both stack modes, and
the ordering edge Docker implies is added to `depends_on`.

`ipc:` and `pid:` are answered with measurements instead of a blanket "unsupported": every box
already has a private IPC and PID namespace, so `private` is reported as ALREADY ENFORCED, and two
members of one stack were measured to share only their network namespace, which is what makes
`pid: service:X` unsatisfiable here rather than merely unimplemented.

`tmpfs:` options are applied. `size=`, `mode=`, `noexec` and `ro` now reach the mount; `nosuid` and
`nodev` are kern's floor and a `suid` or `dev` token is named as recognised-and-never-applied rather
than acted on. A long-form `{type: tmpfs}` volume entry, which used to be dropped with a pointer to
`--tmpfs`, is now translated into one.

`kern docker run` follows: `--device`, `--dns`, `--dns-search`, `--dns-option`, `--dns-opt` and
`--log-opt max-size/max-file` are translated instead of refused, `--device` through the same
normaliser the compose parser uses so the two surfaces cannot disagree. Any other `--log-opt` is
still refused rather than dropped.

**`networks:` is now enforced by default, and `internal: true` is a real boundary for the first
time.** A compose file whose networks leave two services with nothing in common gets one network
namespace PER SERVICE, so the separation it asked for is enforced by the absence of a relay rather
than dropped. kern says so at bring-up, names the pairs it separated, and `--pod` keeps the old
single-namespace wiring for anyone who prefers the speed. A file whose networks separate nothing
keeps the pod and is told nothing, because nothing is lost.

Making that usable needed egress per service, which did not exist: outside a pod a box held only
`lo` and could not reach the internet at all. kern now attaches a rootless NAT to each service's own
namespace, through the same `pasta` machinery the pod has always used, at the one instant it is safe
to: the service is held at its pre-exec gate with every namespace built and no instruction run, so a
workload never observes a namespace that has no route one moment and a route the next.

`internal: true` then means what Compose says it means. A service confined to internal networks gets
NO NAT, so there is no route out of its namespace rather than a filter that has to stay correct.
Measured on one stack, one run: the service on a public network reported two routes and reached
`1.1.1.1:443`, the confined one reported zero and could not. A service with `restart:` gets no NAT
either way and is named at bring-up: it is installed as a systemd unit, so `up` never holds it.

**Docker Compose compatibility, measured before and after on the same neutral corpus** of 259 files,
one per repository, sampled across 733 repositories: the share of files kern runs with NO behavioural
difference from what the file says went from **14% to 94%**. What remains is dominated by keys that
ask kern to be less confining than it is (`privileged: true`, `security_opt`) and by
`network_mode: host`, which one namespace per stack cannot express.

**A PUBLISHED PORT NOW BINDS `0.0.0.0`, NOT `127.0.0.1`. Read this one.** `-p 8080:80` and a compose
`ports: "8080:80"` bind every interface, which is what Docker does and what a file written for Docker
means. Until now kern bound loopback and warned; a stack that looked published was reachable only
from the machine it ran on.

This is a deliberate change of a security-relevant default, and the reason is measured. On a neutral
corpus of 259 compose files, one per repository, sampled across 733 repositories, **203 files (78%)**
published a port and therefore behaved differently under kern than their own text says. It was by a
wide margin the largest source of difference: the next cause was worth 75 files, and every key kern
refuses on purpose (`privileged`, `security_opt`) was worth 8 together. Closing it moved the share of
files with zero behavioural difference from **14% to 63%** on that corpus.

The previous posture is one line, and it is stronger than the old default was:

```toml
[kern]
publish_bind = "127.0.0.1"
```

That key is a CEILING, not a default: it overrides even a spec that explicitly writes
`0.0.0.0:8080:80`, so a compose file obtained from anywhere cannot decide where the host listens. It
is read only from the default config, never from a `--config` a compose file named. The box reports
how many specs it narrowed, so the policy is never silent. If `kern.toml` cannot be parsed, kern
publishes on loopback and says so rather than assuming the wide answer.

**Four more compose keys are applied.** `mem_reservation` becomes cgroup `memory.low` through a new
`--memory-reservation` (a soft floor, never a cap and never an OOM kill); `cpu_shares` becomes
`cpu.weight` through a new `--cpu-weight`, CONVERTED between the two scales so Docker's normal (1024)
lands on cgroup v2's normal (100) rather than on 39; `cpu_quota` + `cpu_period` are divided into
`--cpus`, which is the same `cpu.max` line written the short way; `pull_policy` becomes `--pull`.

**`networks:` became a boundary instead of a warning, under `--no-pod`.** A stack in a pod is one
network namespace and cannot segregate anything, so there the key is still reported as dropped.
Without a pod each service has its own namespace and reachability is built edge by edge out of
relays, so the memberships now decide which edges exist: two services with no network in common get
no relay AND no entry in each other's `/etc/hosts`, so the peer's name does not resolve. Measured on
a three-service stack, the plan drops from six relays to four and the two cut directions answer
`nc: bad address` while every shared-network pair still delivers its payload. The boundary is the
absence of a relay rather than a filter, so there is no rule that can be misconfigured open.

A service with no `networks:` key is on the implicit `default` network and is therefore separated
from the services that name one. That is the Compose Specification's rule, and measured over 240 real
compose files it is the dominant case: 52 files have at least one pair that loses its edge, 40 of
them through exactly this. `up` names every cut pair with both memberships before starting anything,
because a removed edge otherwise appears minutes later as `bad address '<peer>'` in a service log,
indistinguishable from a typo or a dead peer.

**`internal: true` now says something different in each wiring, and both are measured.** In a pod it
is all-or-nothing and becomes `--no-outbound` only when every service qualifies; outbound otherwise
stays open (measured: a pod member reaches `1.1.1.1:443`). Without a pod every service is confined
already (measured: the box holds only `lo`, the same connect is refused), so the key is satisfied for
the services that asked and for the ones that did not. The over-application is stated rather than
left to be discovered by a stack that calls an external API.

**CLI surface: five flags added, none changed or removed.** `--dns`, `--dns-search`, `--dns-option`,
`--log-max-size`, `--log-max-file`. Additive, so nothing that runs today stops running.

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
