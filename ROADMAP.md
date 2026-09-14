# Roadmap and known gaps

What kern does not do. Nothing here is a commitment or a date. Shipped work is in
[CHANGELOG.md](CHANGELOG.md); the design is in [ARCHITECTURE.md](ARCHITECTURE.md).

## Under consideration

| | where it stands |
|---|---|
| **GPU slices** | Nothing caps a GPU. `kern doctor` prints the tier a cap would have: `TIER-HW` where a MIG or SR-IOV partition exists, `TIER-SOFT` everywhere else, where a quota is bypassed by skipping the vendor library |
| **A VM tier** | A shared kernel is the wrong boundary for genuinely hostile, multi-tenant code, and [SECURITY.md](SECURITY.md) says so rather than arguing. A microVM tier is the honest answer to that case. Nothing ships, and it would be its own thing rather than a flag on a box |
| **The OCI runtime spec** | Reading and writing the bundle format (`config.json`, the lifecycle verbs) is under consideration. Being a CRI implementation, or the `--runtime` under podman or a kubelet, is not: the format yes, the component position no |
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
| The progress gate classifies by module, not by meaning | Three diagnostics in the image-cache path were silenced under an SDK: a damaged cached entry was re-fetched without saying so. A suite caught it, the gate did not | A rule that reads what a line says, and does not fire on true progress |
| `--egress-allow` cannot be validated on most hosts | On a host with policy routing the defective version passes the case exactly as the fix does, so `acceptance-matrix.sh` reports that it validated nothing rather than a green tick | A board without policy routing |
| Landlock is absent on every ARM board tested | `--landlock-rw` REFUSES rather than running unconfined. Measured absent on Raspberry Pi OS 6.6, Jetson 5.15-tegra and Arduino UNO Q 6.16 | The kernel shipping the LSM. `kern doctor` reports the ABI |
| A host delegating `memory` but not `pids` | Would take the default `TasksMax=512` silently. Not observed anywhere tested | A predicate. None exists yet because a wrong one warns on healthy hosts |
| `kern ps` prints the mapping recorded at start | A forwarder killed while its box keeps running would still show | A live probe. The forwarder is a child of the box's supervisor and dies with it, so the window is narrow |
| Three legacy fallbacks in the pod store reason about a pid the examined process can influence | `is_holder_argv` (argv, forgeable), the `pasta_to_signal` fallback (`comm`), and `pod_boot_is_current` treating an absent `boot` record as this boot. Each is reachable only from a pod dir written by an older kern | ONE condition phrased against the STORE FORMAT: when a pod dir lacking `boot` can no longer be produced by a supported version, all three go together. What reaches the argv fallback, and how the forgery was verified by construction, is in `pod.rs` beside the code |
| Whether a survivable denial helps an attacker | Eleven denied syscalls return `ENOSYS` so probing software falls back. The errno leaks nothing; whether a cheaper map of the filter is worth anything to code already executing is not measured | `SECCOMP_RET_USER_NOTIF` keeps the fallback and hides the structure, but the listener must be the box's parent and fail closed |
