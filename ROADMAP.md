# Roadmap and known gaps

What kern does not do. Nothing here is a commitment or a date. Shipped work is in
[CHANGELOG.md](CHANGELOG.md); the design is in [ARCHITECTURE.md](ARCHITECTURE.md).

## Under consideration

| | where it stands |
|---|---|
| **GPU slices** | Nothing caps a GPU. `kern doctor` prints the tier a cap would have: `TIER-HW` where a MIG or SR-IOV partition exists, `TIER-SOFT` everywhere else, where a quota is bypassed by skipping the vendor library |
| **More governed resources** | I/O bandwidth and IOPS ship and bind where the host delegates `io`. Widening that, plus network shaping |
| **Snapshot and warm start** | Rootless CRIU needs a capability and seccomp suspended, so it would be opt-in and same-host |
| **macOS** | No native port, and a non-goal. A Mac runs the ordinary Linux kern in a Linux VM. Under consideration: a shim so `kern` can be typed on the macOS side |

**Out by design:** network segmentation between services, `deploy.replicas`, `docker.sock`, the
compose `privileged:` key, an automatic fallback on a port collision, and a per-box seccomp profile
from a file. A stack is one pod, and an arbitrary OCI profile is a parser whose bugs permit rather
than crash.

## Known gaps, and what would settle them

| gap | what it costs you | what would settle it |
|---|---|---|
| The progress gate classifies by module, not by meaning | Three diagnostics in the image-cache path were silenced under an SDK: a damaged cached entry was re-fetched without saying so. Found by a suite, not the gate | A rule that reads what a line says. Refusing anything naming a repair would fire on true progress |
| `--egress-allow` cannot be validated on most hosts | On a host with policy routing the defective version passes the case exactly as the fix does, so `acceptance-matrix.sh` reports it validated nothing rather than a green tick | A board without policy routing |
| Landlock is absent on every ARM board tested | `--landlock-rw` REFUSES rather than running unconfined. Measured absent on Raspberry Pi OS 6.6, Jetson 5.15-tegra and Arduino UNO Q 6.16 | The kernel shipping the LSM. `kern doctor` reports the ABI |
| `pasta` refuses to start on WSL2 | A pod comes up loopback-only: services reach each other by name, egress is missing | Why it is refused there and granted on every Linux host tested is not established |
| A host delegating `memory` but not `pids` | Would take the default `TasksMax=512` silently. Not observed anywhere tested | A predicate. None exists yet because a wrong one warns on healthy hosts |
| `kern ps` prints the mapping recorded at start | A forwarder killed while its box keeps running would still show | A live probe. The forwarder is a child of the box's supervisor and dies with it, so the window is narrow |
| Whether a survivable denial helps an attacker | Eleven denied syscalls return `ENOSYS` so probing software falls back. The errno leaks nothing; whether a cheaper map of the filter is worth anything to code already executing is not measured | `SECCOMP_RET_USER_NOTIF` keeps the fallback and hides the structure, but the listener must be the box's parent and fail closed |
