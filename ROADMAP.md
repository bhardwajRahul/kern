# Roadmap and known gaps

One file, because they are one question: what kern does not do. Nothing here is a commitment or a
date. Shipped work is under [Status](README.md#status) and in [CHANGELOG.md](CHANGELOG.md); the design
is in [ARCHITECTURE.md](ARCHITECTURE.md).

## Directions under consideration

Some may never ship, if shipping them would change what kern is.

- **GPU slices.** A workload gets a slice of a GPU rather than the whole device. Not shipped, but the
  judgement is: `kern doctor` reads each GPU from sysfs, read-only, and prints the tier a cap would
  have. `TIER-HW` where a MIG or SR-IOV partition is present, enforced by the device, with kern saying
  it read the partition's presence and has not measured the VRAM split; `TIER-SOFT` everywhere else,
  where a cooperative quota is bypassed by any tenant that skips the vendor library and buys density
  and fairness only.
- **More governed resources.** I/O bandwidth and IOPS caps ship (`vdisk:` `--bandwidth`/`--iops`, box
  `--io-weight` to cgroup `io.max`/`io.weight`) and bind where the host grants both the delegated `io`
  controller and the ext4-on-loop vdisk backend; without those a rootless box reports them unapplied
  rather than pretending. Widening that, plus knobs like network shaping.
- **Snapshot and warm start (CRIU).** Same-host checkpoint and restore of a warm box. Gated: rootless
  CRIU needs a capability and the seccomp filter suspended, so it would be opt-in, same-host, non-GPU.
- **macOS.** No native port, and a non-goal: a daemonless kernel and cgroup sandbox has no macOS
  equivalent. A Mac already runs the ordinary Linux kern inside a Linux VM, verified on Apple Silicon
  with an Ubuntu 24.04 guest ([docs/INSTALL.md](docs/INSTALL.md)). Under consideration is only a thin
  shim so `kern` can be typed on the macOS side, as `kern.exe` is on Windows. It would reach no GPU:
  Apple exposes no compute device to a Linux guest.

**In progress.** A watcher over a stack's whole member set, surviving one supervisor being killed; a
service with a `restart:` policy is already restarted by its own supervisor.

**Deliberately out, not missing.** Network segmentation between services, `deploy.replicas`,
`docker.sock` and the Engine API, and the compose `privileged:` key. These follow from rootless,
daemonless, and one pod as the unit of isolation. (`kern box --privileged` exists and relaxes exactly
five syscalls for nesting; see [SECURITY.md](SECURITY.md).)

> A stack is one pod. Within that model kern is complete: what is out is a consequence of the model,
> not a gap in it.

## Known gaps, and what would settle them

Each entry says what it costs you and what would settle it.

Peer relays for `--no-pod` used to be listed here while they were being built. They ship, and how
they work is in [ARCHITECTURE.md](ARCHITECTURE.md#peer-relays-when-a-stack-gives-each-service-its-own-namespace):
the two-process design, what a shared port costs, the privilege both halves shed, edge rebuilding, and
the published-port round trip.

### The egress fix cannot be validated on most hosts

`--egress-allow` raises the box's loopback itself, so readiness means reachable rather than bound.
Policy routing predicts which hosts needed that: on all six measured, whether `ip rule list` works
(`CONFIG_IP_MULTIPLE_TABLES`) matches whether a bind in a fresh netns succeeds with the loopback down.
The gap is in the checking: on a host WITH policy routing the defective version passes the case
exactly as the fix does, so `scripts/acceptance-matrix.sh` prints that it has validated nothing rather
than a green tick. Settling it needs a board without policy routing, a Jetson or an Arduino UNO Q.

### Falling back automatically on a port collision

`kern compose up` refuses a stack whose services bind one container port, and the refusal states what
`--no-pod` would do: lose one direction, both or neither, with kern saying which once they run.
Automating it is not taken, because the collision is detected before anything starts, where the answer
cannot yet be measured: the fallback would have to start the stack to find out, and a user who wanted
a pod would get a silently different topology.

### A flat build copies the base, and without copy-on-write that is the whole base

Where the unprivileged overlay mount is refused, `kern build` copies the base rootfs and mutates it.
`cp -a --reflink=auto` makes that a metadata operation on btrfs, xfs or bcachefs; on ext4 the whole
base is read and written (measured in the field: 2m49s and 1.9 GB for a build whose only instruction
after `FROM` was an `echo`). The build line says which happened while it happens. Caching a flattened
base would not help, since the base already sits extracted and is copied because the build mutates it;
hard-linking would corrupt every later build on the first in-place `RUN`, and the safe version of that
trick is the overlay this path exists because it cannot use. A limit, not a task.

### No custom per-box seccomp profile from a file

Docker takes `--security-opt seccomp=<profile.json>`; kern picks between the shipped allowlist and the
opt-out denylist (`KERN_SECCOMP=denylist`). A deliberate hold: an arbitrary OCI profile is a general
parser plus a compiler to cBPF, which is what `libseccomp` exists to do because it is easy to get
subtly wrong, and a bug there does not crash, it silently permits. It earns a pinned parser and an
exhaustive proof, the bar the allowlist met. Meanwhile `--cap-drop` narrows per box, and the default
allowlist is already the stricter of the two filters.

### Whether a survivable denial helps an attacker is not known

Eleven denied syscalls return `ENOSYS` rather than killing the caller, so software probing for an
optional fast path falls back ([SECURITY.md](SECURITY.md) has the set). Measured: the errno leaks
nothing, a denied `io_uring_setup` and an unimplemented number both being `-1 ENOSYS`. Not measured:
whether a cheaper map of the filter is worth anything to an attacker who already has code execution.

One lever is moving those eleven to `SIGSYS`, losing the fallback. `SECCOMP_RET_USER_NOTIF` breaks
that trade: a per-box listener answers by policy, so the fallback survives, the errno stops being
deducible from the filter's structure, and each attempt becomes loggable. Not shipped, by decision: a
listener is a process outside the box, so it must be that box's own parent bound to its lifecycle, and
the notify fd must fail closed (verified: a listener-less `USER_NOTIF` filter hangs the workload on
its first `write`), so a dead listener must reap the box.

### Landlock is gated on the kernel, and the flag is fail-closed

A box passing `--landlock-rw` on a kernel without the Landlock LSM is REFUSED rather than run
unconfined, joining `--require-limits` and `--apparmor` in the enforce-or-do-not-run family; boxes not
passing it are unaffected. The open part is availability: measured absent on all three ARM boards
(Raspberry Pi OS 6.6 reports `capability` as its only LSM, Jetson 5.15-tegra, Arduino UNO Q 6.16).
`kern doctor` reports the ABI, and gating on it keeps one script working across a mixed fleet.

### `--ssh` needs `newuidmap`, and a fresh board does not ship it

sshd's privilege separation needs more than one uid in the box's user namespace, so `--ssh` requires
the `--uid-range` path: the setuid `newuidmap`/`newgidmap` helpers plus `/etc/subuid` and
`/etc/subgid` allocations. Measured absent on stock Raspberry Pi OS, the Arduino UNO Q and the Jetson
Orin Nano. kern warns before the box starts and names the fix but cannot install the helper, and what
the ssh CLIENT prints (`kex_exchange_identification: Connection closed by remote host`) says nothing
about uid maps. Installing `uidmap` was enough on a Pi 5.

### `pasta` refuses to start on WSL2

A pod there comes up loopback-only and kern reports why: `Couldn't open user namespace
/proc/<pid>/ns/user: Permission denied`. Running as uid 0 inside the distro is not enough, and why it
is refused there and granted on every Linux host tested is not established. Bounded: services still
reach each other by name, only egress is missing.

### A host that delegates `memory` but not `pids` says nothing about the task ceiling

The uncapped-host notice is driven by `memory_cap_enforceable()`, which covers the case that occurs, a
kernel booted without `cgroup_enable=memory` delegating neither. A host delegating `memory` and
withholding `pids` alone would take the default `TasksMax=512` silently. Not observed anywhere tested,
and no predicate exists yet, because the cost of getting it wrong is a warning on healthy hosts.

### `KERN_MAX_CONCURRENT` is a guard rail, not a resource boundary

The count-and-claim runs under the claims-dir `flock`, so a racing burst can no longer overshoot `N`.
What remains is scope, by design: it bounds the NUMBER of live boxes a cooperating starter admits and
a caller can unset it, whereas `KERN_FLEET_MEMORY_MAX` and `KERN_FLEET_PIDS_MAX` are cgroup limits.

### `kern ps` prints the mapping recorded at start, not a live probe

The forwarder binds its host socket before `kern box` prints "started", and a bind that fails refuses
the box; what `ps` shows afterwards is the registry entry. A forwarder is a child of the box's
supervisor and dies with it, so the gap is narrow: one killed by hand or by the OOM killer while its
box keeps running would still show.

### The release binary trades panic diagnostics for size

The published Linux binaries use a pinned nightly with `-Zbuild-std=std,panic_abort`,
`-Zbuild-std-features=optimize_for_size` and `-Cpanic=immediate-abort`. The cost: a panic prints no
file and no line, so a bug that can only reach one aborts with a bare `SIGABRT`. kern's production
code is panic-free (audited, no abort across the extreme and four-kernel suites), but audited is not
proven. An x86_64 size reproduces across machines, this desktop's build of a tagged commit matching
the published tarball byte for byte; an aarch64 one cannot, a native CI build and a cross build being
different link jobs. The source stays 100% stable Rust, so `cargo test` runs on the same source the
release ships, and the pinned nightly needs a deliberate bump plus re-validation.
