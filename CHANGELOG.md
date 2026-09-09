# Changelog

**CLI stability.** Since v0.7.0 the verbs, their flags and the `--json` shapes change incompatibly
only on a minor bump, never on a patch, and only after a deprecation entry here one release earlier.
`--json` is additive, so consumers must ignore unknown fields. A `cli_surface_is_frozen` test fails
the build on any undocumented change. Full detail for any entry is in the git history.

## Unreleased

**`tty` said "not a tty" inside every alpine box, and the terminal now has a name.**

`kern box -it` and `kern exec -it` allocated the PTY pair on the HOST and handed the slave to the
box as its stdio. The box mounts its own private devpts at `/dev/pts`, which does not contain that
node, so inside the box:

```
isatty(0)                 1              the fd IS a terminal
readlink /proc/self/fd/0  /dev/pts/2     the HOST's path
stat /dev/pts/2           ENOENT         that path does not exist in the box
```

musl's `ttyname_r` is exactly that readlink, a `stat` and a device comparison, with no second
strategy, so it returned ENOENT. glibc's falls back to scanning `/dev`, where it found the
`/dev/console` bind kern already installed, and reported `/dev/console`. So the same box looked
correct on a Debian image and broken on an alpine one, which is the image most people run. podman
does not have it: `podman exec -it c sh -c tty` prints `/dev/pts/0`, because its slave comes from
the container's devpts.

The pair is now opened from the BOX's `/dev/ptmx` in the process that holds the box's mount
namespace, and the master is passed back to the CLI over a socketpair with `SCM_RIGHTS` (a pipe
cannot carry a descriptor). Both libraries then read `/dev/pts/0` and the node is there. Measured,
same probe, before and after:

```
box -it, musl     ttyname_r FAILED rc=2      ->   /dev/pts/0
box -it, glibc    ttyname_r = /dev/console   ->   /dev/pts/0
exec -it, musl    ttyname_r FAILED rc=2      ->   /dev/pts/0
```

The host pair is still opened and is still the fallback: a box that cannot produce a terminal of its
own (no devpts, a `--rootfs` kern did not populate, a kernel that refused the mount) behaves exactly
as it did before. A terminal is a convenience, and failing a box over one would be a worse defect
than the one being fixed. `/dev/console` is kept for that case, and the comment that used to call it
the fix now says which library it was ever the fix for.

`scripts/certify-ttyname.sh` is the certificate. It builds its own static probe, so it runs from a
fresh clone with no fixture, and it is RED on the previous binary rather than merely green on this
one. Measured on six distributions, three of them SELinux Enforcing, all passing both `box -it` and
`exec -it`: Fedora 44 (6.19), CentOS Stream 10 (6.12), Rocky Linux 10.2 (6.12), Debian 13 (6.12),
openSUSE Leap 15.6 (6.4) and Ubuntu 24.04 (6.8).

Those VMs found a second defect this developer's host could not: `libc::ioctl` takes a `c_ulong`
request against glibc and a `c_int` against musl, so the first version of the new code compiled
cleanly here and failed to build for `x86_64-unknown-linux-musl`, which is the target kern ships.

**A `kern doctor` row that denies a memory cap now names the cgroup it probed, and `kern inspect`
reports the cap the kernel actually holds.**

An outside reviewer held a review on this. doctor said a `--memory` write is "accepted and silently
never bites", and on the same host a box started with `--memory 64m` reported `memory_max` 67108864
and an `exec` that overran it exited 137. Two statements, no way to tell whether they were even about
the same cgroup, because neither named one.

Both halves are now checkable rather than asserted:

- Every doctor row that says a cap does not bind names the directories the write-probe used, taken
  from the same call the probe resolves its targets with, so the row can never name a directory the
  probe did not touch. Its remedy also says how to check it against a running box.
- `kern inspect --json` gains `memory_max_enforced`, read back from the box's own cgroup. The
  existing `memory_max` is the value the box was STARTED with, echoed from the registry, which is
  what the reviewer read as a cap in force. `null` means nothing is holding it. The human row says
  `(requested, NOT enforced here)` in that case. `--json` is additive by contract.

What the two observations do NOT establish, and the code no longer claims: the cgroup v2 root has no
`memory.max` file at all, so a box whose PID 1 is in `0::/` has nothing capping it, and exit 137 is
SIGKILL, which a system OOM kill delivers identically. A kill by kern's own cap always prints kern's
own OOM message. The comment that had recorded "the cap bound; the probe said it would not" as a
settled fact records the measurement instead.

Separately, the probe already asked BOTH directories a box can be capped in; previously it asked
`kern.slice` and stopped, and on that reviewer's host reported on a directory no box went near.

The `mem-cap` row's four cases moved out of `inspect`'s 245-line body into a pure `mem_cap_row`,
because the branch that matters most is the one this developer's host cannot produce: `enforced ==
None` needs a box outside every kern leaf. Buried in an I/O function it was unreachable by any test,
which is why an outside reviewer had to find it by hand. It is now pinned by four tests, each verified
to go red under mutation of its own arm.

`docs/RESOURCES.md` promised its way past the same host class: it said both verbs "carry a default
memory cap of 512 MiB whether or not you ask for one", and named the two mechanisms that deliver it
without naming the host where NEITHER exists. It now says so, and shows the two `inspect` fields and
the 137 caveat.

**`kern run` no longer pays for a systemd scope it does not need: 4.70 ms to 0.87 ms.**

`kern run` bought its resource caps with a transient `systemd-run --user --scope`, one per
invocation. `kern box` stopped doing that long ago and caps directly under kern's delegated
`kern.slice`; `run` was excluded by a single flag, and the scope was the entire difference between the
two verbs. Measured on one desktop, the shipped extreme binary, 200 samples per column, medians:

```
kern run -- /bin/true                    4.700 ms
KERN_NO_SCOPE=1 kern run -- /bin/true    0.577 ms   (the same work, no scope, no caps)
systemd-run --user --scope -- /bin/true  3.867 ms
```

The scope was 4.12 ms of the 4.70. Paired and alternating sample by sample against the previous
binary, `kern run` is **-3.3 to -4.0 ms** depending on how busy the user manager is (two runs, 300
pairs each: -4.004 ms, 95% interval [-4.074, -3.940], and -3.335 ms, [-3.384, -3.256]); `kern box` is
unchanged, its interval containing zero. The whole surface on that host, same binary and recipe as a
release, 400 samples each: `exec` 0.79 ms, `run` 0.87 ms, `box --rootfs` 2.39 ms, `box --image alpine`
3.38 ms, against bubblewrap 0.9.0 at 2.72 ms for the same namespaces and rootfs.

**The tail moved more than the median, and it is the better half of the result.** Over 400 samples,
`kern run`'s p99 goes from 5.735 ms to 1.267 ms and its worst sample from 15.304 ms to 1.487 ms. A
D-Bus round trip to a shared, single-threaded user manager is a queue, and a queue is what a tail is
made of.

**It also scales now, where it did not before.** `kern run -- /bin/true` in parallel, amortised
runs per second:

```
concurrency      1      8     32    100    200
before         214    496    288    151     86
this branch   1053   4734   4891   4601   4052
```

The old path got SLOWER as concurrency rose, because every run serialised on the same manager. Zero
failures on either column, and under a 200-way burst 400 out of 400 runs read exactly `67108864` from
their own `memory.max`, so nothing about the cap is traded for the throughput.

The reason recorded for excluding `run` was that it `exec()`s the workload in place, leaving no
process behind to remove the cgroup the way the scope's `--collect` does. That had stopped being true:
the scope path itself forks a proxy, so a `kern run` was already `caller -> kern -> workload`. The
fork is now performed on the direct path too, and the parent is what removes the leaf. **The process
topology is unchanged**, and so is everything a caller can observe: the command's exit code, a
forwarded Ctrl-C (130), the OOM message on a 137, stdin and stdout, and `--landlock-rw` applied to the
command and not to kern.

Two behaviours that are new and worth knowing:

- `kern run`'s cgroup is now `kern-run-<pid>`, not `kern-box-run-<pid>`. With the box prefix, a live
  `kern run` was reported by `kern ps` as a box with no registry record that `kern stop` could not
  reach. A box may legitimately be named `run`, so a reserved tag could not have separated them.
- The orphan sweep reaps a `kern run` leaf but **never kills what is still inside it**. A box's
  processes are the box's; `kern run` governs processes the caller started, and a backgrounded child
  outliving the launcher is an ordinary use. Under the scope it kept the scope alive and was collected
  when it exited; the sweep now leaves it alone and removes the directory once it is empty. Verified
  against a real cgroup: a survivor of a SIGKILL'd launcher is still running after a `kern box`, a
  `kern gc` and a `kern run`, and its directory goes only once it exits.
- `kern run` now runs that sweep itself, in the parent, WHILE the workload runs. Nothing on this
  verb's path ever called it, so a machine that only ever ran `kern run` accumulated one directory per
  killed launcher until a `kern box` or a `kern gc` came along. It is free: with 130 entries planted to
  be stat'd and skipped, the paired difference against the same binary without the call is -7.9 us.

Where kern's `kern.slice` is not usable - no systemd user manager, `KERN_NO_SCOPE=1`, inside a box, a
`kern build` RUN step, a `--restart` unit - `kern run` takes exactly the path it took before.

**The fork is not gated on the systemd path, and the first version of this change got that wrong.**
An outside reviewer measured the shipped v0.9.31 on two hosts, 200 sequential `kern run` each:

```
Ubuntu, systemd user manager present    0 leftover cgroup dirs -> 200 runs -> 0
WSL2 Alpine, no user manager            2                      -> 200 runs -> 201
```

The host that leaks is the one WITHOUT a manager, because there the leaf is a plain directory kern
made and `exec()` in place leaves nothing to remove it. The host that does not leak is the one with a
manager, where the leaf lives inside the transient scope and systemd reaps the whole unit. Gating the
fork on the direct path therefore fixed the case that was already fine and left the broken one alone,
and the broken one is the configuration of the WSL rootfs this project publishes. The fork is now
decided by `run_should_fork`, a pure function of two facts: a capped leaf exists, and nothing outside
already supervises the workload. On a host with an outer enforcer - kern's own scope proxy, a
`--restart` unit, a build step - it still `exec()`s in place, because that proxy already waits,
forwards signals, reports the OOM and propagates the code.

The same host also lost its OOM diagnostic: with no process outliving the workload, a run killed by
the 512 MiB default nobody typed printed the shell's bare `Killed`. A supervisor restores the message
there for free.

**Two notices were wrong and are fixed with it.** The check behind "this command runs with no RAM
ceiling" read `/proc/self/cgroup`, which is the SUPERVISOR's cgroup once `kern run` forks, and that
one is uncapped by construction: every plain `kern run` would have printed the notice over a workload
holding exactly 536870912. And the notice named `KERN_NO_SCOPE` as the cause whatever the cause was,
so a host with no user manager was told to unset a variable it never set. It now names the reason it
can actually name.

**`kern run` also sweeps the cgroup it can actually reach.** The per-start sweep only ever looked in
`kern.slice`, the one directory a host without a user manager does not have, so nothing on that path
ever swept anything. It now sweeps the caller's own cgroup as well, deduplicated where they coincide,
and it runs in the parent while the workload does, so it costs nothing measurable: paired against the
same binary without it, `kern run` differs by -2.9 us and `kern box` by -0.0 us, both intervals
containing zero.

**`/dev/tty` stays out of a box, and it can no longer be created by accident.** The device is excluded
because a controlling terminal enables TIOCSTI-style injection on unhardened kernels, and that reason
was re-measured rather than trusted: under a real pty, a box started WITHOUT `-it` inherits the
launcher's controlling terminal, `tty_nr=34816` for both, so the device inside the box would be the
operator's own terminal. What was wrong is that `/dev` is a tmpfs the box's root owns, so a redirect
CREATED a regular file there and a program writing a prompt got no error and wrote into nothing. The
path is now a directory: `open` for writing is EISDIR whether or not `O_CREAT` is passed, so the write
fails loudly and nothing that worked before changes.

**A box that could not be BUILT now says what to check, from either process that reports it.** The
message is printed twice in kern, and one of the two could never carry a hint: `report_exec_failure`
runs in the forked child and `_exit`s, so the error never reaches the CLI's hint function. A reviewer
measured `kern: sandbox setup failed: mount(overlay) failed: Invalid argument (os error 22)` arriving
bare while every neighbouring branch carried advice. The remedy is one constant used by both, naming
the four things a setup failure is (the mount, the uid map, the seccomp filter, the AppArmor profile)
and pointing at `kern doctor`, which reports all four. The older wording named two of the four.

**A box whose kern binary was replaced is visible again to the channel that exists to find it.** The
kernel appends `" (deleted)"` to `/proc/<pid>/exe` once the file behind a running process is gone,
which is the state of every already-running kern the moment an upgrade overwrites the binary.
`live_box_supervisors_via_proc` compared against the bare name, so those processes read as not-kern
and their boxes disappeared from the fallback channel that exists precisely to find boxes the registry
has lost, on the one event most likely to lose them. Found on this desktop as a disagreement between
the two channels in the same second: the cgroup channel reported a box the `/proc` channel did not.

**`kern doctor` asks about both directories a box can be capped in.** `apply_limits` caps under
`kern.slice` on the direct path and under the caller's own cgroup otherwise; the probe used
`or_else`, which reaches the second only when the first is absent, so on a host where the slice exists
but boxes do not use it the probe reported on a directory no box goes near. An outside reviewer
measured the consequence: doctor said a `--memory` write "silently never bites" while a box on the
same host held `memory_max = 67108864` and an exec that overran it was killed with 137. Both
directories are asked now, and the better answer wins.

**The cap probe's leftovers are reaped.** Its throwaway child is removed on every path the probe
returns from, so one survives only when the process died in between; nothing collected those, because
the sweep knew only the two box prefixes. Measured: one `kern-capprobe-*` in `kern.slice` from a pid
long gone, which four consecutive `kern doctor` runs did not add to and `kern gc` did not remove. The
sweep knows the family now, and never kills anything in it, because it is empty by construction.

**`kern exec` no longer refuses when there is no cap to escape.** The fail-closed asked "was the
command placed in the box's cgroup" and never "is there a cgroup, carrying a real limit, to be outside
of". On a host with no delegation `apply_limits` returns `None`, the box sits in the caller's own
cgroup, the placement has nothing to place, and `kern exec` refused with 126 while telling the operator
the command would run outside `--memory`/`--pids` caps the box did not have. Reported by an outside
reviewer on a box that was NOT at its pids limit.

The function that answers the real question was already there and its result was thrown away: the block
that called it read `let placed = true; if !placed`, dead since the migration was replaced by
`clone3`. It is consulted now, before the `setns`, because it reads `memory.max` and `pids.max` through
the cgroup's descriptor and the same read through a path afterwards reports a capped box as uncapped.
A box at its `--pids-limit` still refuses, because a pids ceiling is a real limit.

**`kern doctor` names the way through, and the health probe stopped writing into a hole.** The
refusal points at `kern doctor` to tell its two causes apart; the row it points at did not name
`KERN_ALLOW_UNCAPPED`, so a reader who followed the pointer learned which cause they had and not what
to do about it. It names it now, along with the verb that refuses and the probe that does not.

The probe's own notice is gone rather than kept, and the reason is measured: a marker written to
stderr from inside the probe's child appears in a foreground `kern exec` and does NOT appear in
`kern logs` or in the box's log file, which stays 0 bytes. A detached supervisor's stdout and stderr
are one pipe that nothing reads. A notice nobody can read is the silent-success shape this codebase
refuses, and keeping it would have been worse than not having it, because it would have looked like
the condition was reported. It is reported, in the one place an operator looks when a host behaves
this way.

**`kern exec` no longer loses a whole class of host to its own fail-closed, and the health check no
longer reports a healthy box as unhealthy there.** The refusal is right: a command that steps around
the box's `--memory`/`--pids-limit` while the operator believes it is capped is the escape it exists
to stop. But it fires for two causes, and only one of them is the box's fault. The other is cgroup v2
delegation containment, which needs write access to the `cgroup.procs` of the COMMON ANCESTOR of the
source and destination cgroups: from a shell in `/init.scope` that ancestor is the root cgroup, so on
WSL2 with `systemd=true` every `kern exec` refused, on hosts where nothing was wrong. No
implementation fixes that one, because a process there cannot reach the user's delegated tree and
cannot move itself into it either.

So the default stays REFUSE and `KERN_ALLOW_UNCAPPED` is the way through, which is the meaning that
variable already carries in `SECURITY.md`, `INSTALL.md` and `RESOURCES.md`: explicitly accept running
uncapped where a cgroup cap cannot be applied. It adds no CLI surface, which matters because that
surface is frozen. The command then runs and says what it gave up; the box's namespaces, seccomp
filter and AppArmor profile still apply to it, and only the resource ceiling does not.

`exec_in_box` has a second caller that nobody had looked at: kern's own `--health-cmd` probe. Refusing
there turned the same host property into a permanent false "unhealthy" for every box with a health
check, with nothing in the health output naming the cause. The probe now proceeds, warning once per
process rather than once per interval. What that costs is stated rather than glossed: on those hosts
the probe runs outside the box's caps, which is the same tradeoff already documented for its baseline
capabilities and its unconfined AppArmor, and on the same narrow set of hosts.

**`kern exec`'s fail-closed refusal now names what to do about it.** It states that the command could
not be placed in the box's cgroup, which is the consequence; it did not say which of the two causes it
was, and they need opposite actions. A reviewer hit it on every `kern exec` on WSL2 with
`systemd=true`, where the previous release ran the command uncapped and warned, and had no way to tell
a box at its `--pids-limit` from a host layout on which no exec can ever join. Both are named now, and
`kern doctor`, which reports which cap path a host takes in its first two lines, is pointed at.

**A fork failure is no longer blamed on user namespaces or the rootfs, for any errno.** That hint has
now been wrong twice, to two reviewers, under two different errnos: EAGAIN from a tightened
`RLIMIT_NPROC`, which got a branch of its own last release, and ENOMEM on WSL2, measured
deterministically. Both times the reader was sent to two places that were already fine, because by
the time any fork on this path runs the user namespace exists and the rootfs has been validated. The
rule is now the CLASS rather than one errno at a time, which is how the third one would otherwise have
been found by a user. It states what the errno is, names the two limits a fork can actually hit, and
does not guess at a cause: `clone3(CLONE_INTO_CGROUP)` into a cgroup it may not write answers EACCES
on the developer's host and EBUSY into a populated one, neither of which is the ENOMEM measured in the
field, and a hint that guessed would be the defect it replaces.

**Finding the delegated slice is not the same as being allowed to enter it, and the difference is a
class of host.** cgroup v2's delegation containment rule needs write access to the `cgroup.procs` of
the COMMON ANCESTOR of the source and destination cgroups, not just of the destination. From a shell
in `/init.scope` that ancestor is the root cgroup, owned by root, so a `kern.slice` that is delegated,
writable and correctly capped is still unreachable. Measured on WSL2 with `systemd=true`: the leaf was
created, both caps were written and read back, and then `clone3(CLONE_INTO_CGROUP)` and the
`cgroup.procs` write both failed. `kern run` warned and ran UNCAPPED where the previous release had
capped it; `kern box`, which is fail-closed on the same placement, would have refused to start at all.
The decision site now asks the kernel's own question with one `access(2)` on that ancestor, so a host
that cannot place takes the systemd scope exactly as it did before.

**And a whole class of hosts could not reach the fast path at all, for a reason that had nothing to
do with the fork.** kern derived its delegated `kern.slice` by walking UP from its own cgroup looking
for a `user@<uid>.service`. On WSL2 with `systemd=true` a user manager is running and the login shell
sits in `0::/init.scope`, whose only ancestors are itself and the root: the search answered "no
delegated slice on this host" while the tree sat one directory away. Measured there, same binary:
`kern run` 11.5 ms with the per-invocation scope against 1.0 ms with the scope skipped. When the
ancestor search finds nothing, kern now tries the canonical `user.slice/user-<uid>.slice/
user@<uid>.service` built from the real uid, and uses it only if the directory is really there. A host
laid out some other way answers exactly what it answered before.

**The numbers above are warm, and the FIRST invocation on an idle machine is not.** Measured on the
same host by removing `kern.slice` before each sample, three samples each: the new binary reads 19.2,
1.6 and 27.9 ms and the previous one 16.2, 23.4 and 22.4. That cost is systemd's, not kern's - a
standalone `systemd-run --user -p Delegate=yes --slice=kern.slice --scope -- true` on the same idle
manager is 19.5 ms on its own - and both binaries pay it. The change moves the warm case and leaves the
cold one where it was.

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
