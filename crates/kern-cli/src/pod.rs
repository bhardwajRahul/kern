//! `kern pod` - a **pod** is a set of boxes that share ONE loopback network, so services reach each
//! other by name on `127.0.0.1` (like a Kubernetes pod). A hidden **holder** process (`kern
//! __pod-holder`, [`kern_isolation::run_pod_holder`]) owns the pod's user+net namespace; each
//! `kern box --pod <name>` box `setns`es into it. A shared `/etc/hosts` (bind-mounted into every pod
//! box) maps each member name → `127.0.0.1`, updated as members join. Pod members are co-trusted
//! (they share the user+net ns); the pod is the network trust unit.
//!
//! **Outbound** is OPTIONAL: if `pasta` (passt) is installed, `create` attaches it to the pod net ns
//! for rootless NAT'd internet access + DNS (unless `--no-outbound`); without pasta the pod is
//! loopback-only (inter-service only; publish to the host with `-p` on a box). kern itself needs no
//! extra dependency to run - pasta only unlocks pod egress.

use crate::error::Error;
use std::io::{BufRead, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

/// `<XDG_RUNTIME_DIR|/run/user/uid>/kern/pods`.
pub(crate) fn pods_root() -> PathBuf {
    crate::registry::assert_registry_child("pods"); // classification chokepoint (see registry.rs)
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    base.join("kern/pods")
}

/// A pod's private directory (`…/pods/<name>`): holds the `holder` pid file and the shared `hosts`.
fn pod_dir(name: &str) -> PathBuf {
    pods_root().join(name)
}

/// Path of a pod's shared `/etc/hosts` (bind-mounted into every member box).
pub fn hosts_path(name: &str) -> PathBuf {
    pod_dir(name).join("hosts")
}

/// Path of a pod's `/etc/resolv.conf` - present only when the pod has OUTBOUND (a `pasta` NAT was
/// set up); bind-mounted into member boxes so DNS works. `None`/absent → loopback-only pod.
pub fn resolv_path(name: &str) -> PathBuf {
    pod_dir(name).join("resolv.conf")
}

/// Is this pod's `pasta` still the live process we recorded?
///
/// Verified by `comm` rather than by liveness alone: passt re-execs into an ISA variant
/// (`pasta.avx2`, never the bare name) and a recorded pid can be reused once it dies. Same guard
/// `teardown` applies below, for the same reason.
fn pasta_alive(name: &str) -> bool {
    let Ok(raw) = std::fs::read_to_string(pod_dir(name).join("pasta.pid")) else {
        return false;
    };
    let Ok(pid) = raw.trim().parse::<i32>() else {
        return false;
    };
    // `/proc/0` does not exist so this would fall out anyway, but a degenerate pid is rejected on
    // purpose here as it is in `holder_pid` and `starter_alive`, rather than by accident.
    if pid <= 0 {
        return false;
    }
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .map(|c| is_pasta_comm(c.trim()))
        .unwrap_or(false)
}

/// The network sentence for an EXISTING pod, for a caller reporting on one it did not just create
/// (a `compose up` that REUSED a pod has no `create` line above its summary).
///
/// A BOOL WAS THE WRONG SHAPE AND IT SHIPPED. The first version of this returned "does it have
/// outbound", and the caller printed "NO outbound (install `passt`/`pasta`)" for every false. So a
/// user on Fedora whose pasta was INSTALLED and refused to start read, two lines under kern's own
/// correct "pasta IS installed but did not start", an instruction to install it. Reported as #6
/// within hours of the release that introduced it. The comment above the caller even said `pod
/// create` distinguishes five states; the code beneath it collapsed them to two.
///
/// So this returns the sentence, not a flag: the wording lives in one place, and a caller cannot
/// map a state onto the wrong message because it never sees the states. "pasta is on PATH" and
/// "pasta is running for THIS pod" are asked separately, because the difference between them is
/// exactly what the bad message got wrong.
///
/// The three facts are read here and decided in [`network_sentence`], which is pure and therefore
/// testable: the previous version reached the filesystem inside every arm, so no test could reach
/// the arms at all, and the wrong-arm defect below shipped unexercised.
///
/// `which_pasta` walks PATH, and is only consulted for the case where nothing else has answered, so
/// it is evaluated lazily rather than on every summary line.
pub fn network_summary(name: &str) -> String {
    let alive = pasta_alive(name);
    let resolv = resolv_path(name).is_file();
    let installed = !alive && !resolv && which_pasta().is_some();
    network_sentence(alive, resolv, installed).into()
}

/// `installed` is consulted ONLY when `!alive && !resolv`; every other arm is decided before it is
/// read, which is why the caller may pass `false` for it without looking.
fn network_sentence(alive: bool, resolv: bool, installed: bool) -> &'static str {
    match (alive, resolv) {
        (true, true) => "services reach each other by name + outbound to the internet (pasta)",
        (true, false) => {
            "outbound is up but DNS is not - the pod can reach an IP and cannot resolve a name"
        }
        // pasta WROTE this resolv.conf, so it started for this pod and has since exited: crashed,
        // OOM-killed, or caught a teardown that raced. The arm below must not absorb this case. It
        // says "the `pod create` line says why it refused", and nothing refused: create succeeded
        // and printed no reason, so that sentence sends the reader to look for an explanation that
        // was never printed. Same defect class as #6's visible symptom, one arm over, in the
        // function written to fix it.
        //
        // No new marker is needed to tell the two apart, because `setup_outbound` only reaches the
        // `resolv.conf` write after every failure path has already returned: the file existing IS
        // the record that pasta once came up, and `teardown` removes the whole dir, so it cannot be
        // left over from an earlier pod of the same name.
        (false, true) => {
            "outbound is DOWN - pasta started for this pod and has since exited (create reported \
             no problem, so look for a crash, an OOM kill, or a racing teardown)"
        }
        (false, false) if installed => {
            "loopback-only - services reach each other; pasta is installed but is not running for \
             this pod (the `pod create` line says why it refused)"
        }
        (false, false) => {
            "loopback-only - services reach each other; NO outbound (install `passt`/`pasta` for \
             egress)"
        }
    }
}

/// The inode of `/proc/<pid>/ns/<kind>` - a namespace's stable identity. Used to detect PID reuse:
/// a recorded holder PID is only trusted if its net ns inode still matches the one from create time.
fn ns_inode(pid: i32, kind: &str) -> Option<u64> {
    std::fs::metadata(format!("/proc/{pid}/ns/{kind}"))
        .ok()
        .map(|m| std::os::unix::fs::MetadataExt::ino(&m))
}

/// The holder PID for pod `name` if the pod exists, its holder is still alive, AND its net ns is the
/// SAME one recorded at create (guards against the PID being reused by an unrelated process after the
/// holder died - otherwise a box could `setns` into a stranger's namespace). Else `None`.
pub fn holder_pid(name: &str) -> Option<i32> {
    let dir = pod_dir(name);
    let pid: i32 = std::fs::read_to_string(dir.join("holder"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    // Same reason as `marker_alive` above: `kill(0, 0)` and `kill(-1, 0)` succeed, so a degenerate
    // value in the holder file would pass the liveness probe. The netns identity check below happens
    // to reject it too, but that is an accident of `/proc/0` not existing, not a guard.
    if pid <= 0 {
        return None;
    }
    if unsafe { libc::kill(pid, 0) } != 0 {
        return None; // holder gone
    }
    // Verify the net ns identity matches what we recorded - reject a reused PID.
    let want: u64 = std::fs::read_to_string(dir.join("netns"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if ns_inode(pid, "net") == Some(want) {
        Some(pid)
    } else {
        None
    }
}

/// The argv token that makes a process kern's own pod holder. One definition: `create` spawns the
/// holder with it and [`holder_to_reap`] recognises it, so the two cannot drift.
const HOLDER_ARGV: &str = "__pod-holder";

/// Does this raw `/proc/<pid>/cmdline` belong to a kern pod holder?
///
/// POSITION, NOT PRESENCE, and the first version of this got it wrong. A cmdline is NUL-separated
/// argv, so matching "any argument equals the marker" already rejects `--flag=__pod-holder` and
/// `__pod-holder-ish`. It does NOT reject `kern box x -- echo __pod-holder`, where the marker is a
/// whole argument belonging to the WORKLOAD, and the test written to assert that case is what caught
/// it. Since the answer decides a `SIGKILL`, presence anywhere is too weak.
///
/// `create` spawns the holder as `<kern> __pod-holder`, so the marker is argv[1] and nowhere else,
/// and argv[0] must name a `kern`. Both are required: argv[1] alone would accept
/// `grep __pod-holder /proc/1/cmdline`, and argv[0] alone accepts every other kern subcommand.
/// argv[0]'s file name rather than its full path, so a holder started by a kern that has since been
/// reinstalled elsewhere is still recognised as one.
fn cmdline_is_holder(cmdline: &[u8]) -> bool {
    let mut argv = cmdline.split(|c| *c == 0);
    let Some(arg0) = argv.next() else {
        return false;
    };
    let base = match arg0.iter().rposition(|c| *c == b'/') {
        Some(i) => &arg0[i + 1..],
        None => arg0,
    };
    base == b"kern" && argv.next() == Some(HOLDER_ARGV.as_bytes())
}

/// Does this pid's argv carry kern's holder marker? IO wrapper over [`cmdline_is_holder`]; an
/// unreadable `/proc/<pid>/cmdline` (the process died, or it is another user's) answers no, because
/// "cannot read it" is not "it is ours".
fn is_holder_argv(pid: i32) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| cmdline_is_holder(&b))
        .unwrap_or(false)
}

/// Is `pid` recorded as the holder of some OTHER live pod? Used to refuse killing a sibling pod's
/// holder that happens to have recycled the number.
fn claimed_by_another_pod(pid: i32, except: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(pods_root()) else {
        return false;
    };
    rd.flatten().any(|e| {
        let n = e.file_name();
        let other = n.to_string_lossy();
        other != except
            && std::fs::read_to_string(e.path().join("holder"))
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
                == Some(pid)
    })
}

/// The holder to KILL, which is a different question from the one [`holder_pid`] answers.
///
/// `holder_pid` asks "may a box `setns` into this?", where any doubt must be a no, and that is right
/// for its callers. Its `None` therefore covers five situations and only two of them mean there is
/// nothing to kill:
///
///   the `holder` file is unreadable   -> a live holder may exist, and is about to become unnameable
///   the pid is <= 0                   -> nothing to kill
///   `kill(pid, 0)` fails              -> the process is already gone
///   the `netns` file is unreadable    -> cannot tell whose it is
///   the recorded netns inode differs  -> provably a stranger, and must NOT be killed
///
/// `teardown` used that `None` and then removed the directory anyway, so "cannot tell" became a
/// holder running for the life of the session with the only record of it deleted. MEASURED, not
/// imagined: one integration run left SEVEN behind, and two of them survived `kern pod rm` because
/// their directory was already gone and nothing could name them again.
///
/// So this identifies the PROCESS rather than trusting the inode alone, and requires two independent
/// facts before signalling: its argv carries kern's holder marker, and no other pod dir claims the
/// same pid. The netns inode stays the fast path; this is only consulted when it could not answer.
fn holder_to_reap(name: &str) -> Option<i32> {
    if let Some(pid) = holder_pid(name) {
        return Some(pid); // identity confirmed by the recorded netns inode
    }
    let pid: i32 = std::fs::read_to_string(pod_dir(name).join("holder"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    // `kill(0, ...)` signals the caller's own process group and `kill(-1, ...)` signals every process
    // it may signal, so a degenerate value must never reach `kill` even to probe liveness.
    if pid <= 0 || unsafe { libc::kill(pid, 0) } != 0 {
        return None;
    }
    // A live pid whose inode check could not be made. Kill it only if it is provably one of ours and
    // provably not somebody else's.
    if is_holder_argv(pid) && !claimed_by_another_pod(pid, name) {
        return Some(pid);
    }
    None
}

/// Is a concurrent `pod create` still mid-startup for this dir? True iff the `starting` marker names
/// a live PID **whose kernel start-time still matches** - so a stale marker whose pid was reused by an
/// unrelated process reads as dead, not as a live starter. Used only to make two racing
/// `pod create <same>` safe: the mkdir loser must not reclaim the dir while the winner's holder is
/// still coming up (its `holder` pid isn't written yet).
fn starter_alive(dir: &std::path::Path) -> bool {
    let Ok(marker) = std::fs::read_to_string(dir.join("starting")) else {
        return false;
    };
    let marker = marker.trim();
    // `pid:starttime` (new) or a bare `pid` (older marker) - parse whichever is present.
    let (pid_s, want_start) = marker.split_once(':').unwrap_or((marker, ""));
    let Ok(pid) = pid_s.parse::<i32>() else {
        return false;
    };
    // `kill(0, 0)` and `kill(-1, 0)` both SUCCEED, so a marker holding 0 or -1 would read as a live
    // process; with a bare-pid marker the back-compat arm below then answers `true` outright.
    if pid <= 0 {
        return false;
    }
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false; // no such live process
    }
    match want_start.parse::<u64>() {
        Ok(s) => crate::registry::proc_starttime(pid) == s, // reject a reused pid
        Err(_) => true, // bare-pid marker: liveness only (back-compat)
    }
}

/// `kern pod create <name> [--no-outbound] [--uid-range]` - spawn the pod's namespace holder + seed its
/// hosts, and (unless `--no-outbound`, and if pasta is installed) attach pasta for internet egress.
/// Publish a service with `-p` on its member box. `uid_range` maps a subordinate uid RANGE into the
/// pod's shared user namespace (via the holder) instead of the single-uid self-map - needed when the
/// pod hosts OCI images that drop privilege in their entrypoint (postgres/redis/…). `kern compose`
/// passes `ImageDefault` when the stack has image boxes and `Requested` when a service asked in as
/// many words; a pod of root-only services stays single-uid (faster, more isolated).
pub fn create_with_range(
    name: &str,
    want_outbound: bool,
    uid_range: kern_isolation::UidRange,
) -> Result<(), Error> {
    validate_name(name)?;
    let dir = pod_dir(name);
    let _ = std::fs::create_dir_all(pods_root()); // ensure the parent exists (recursive)

    // ATOMICALLY claim the pod by a NON-recursive 0700 mkdir: two concurrent `pod create <same>`
    // can't both proceed (the loser gets AlreadyExists). Private (0700): another local user must not
    // read/alter a pod's hosts or holder pid. A leftover dead pod dir (holder gone) is reclaimed.
    if let Err(e) = std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            // A live holder means a real, running pod. A live `starting` marker means a CONCURRENT
            // `pod create` is mid-startup (it won the mkdir but hasn't written its holder pid yet).
            // In BOTH cases refuse: never stomp an in-progress claim, or we'd orphan the winner's
            // holder. Only a genuinely dead leftover (no holder AND no live starter) is reclaimed.
            //
            // The winner writes its `starting` marker microseconds after winning the mkdir - but on a
            // slow host that gap widens to milliseconds, so a naive single check here can race in and
            // see neither holder nor starter *before* the winner marks itself. Poll briefly (bounded)
            // before concluding the dir is dead: a live winner appears within a few ms; a genuinely
            // dead leftover (creator crashed pre-marker) stays empty and is reclaimed after the wait.
            let claimed = || holder_pid(name).is_some() || starter_alive(&dir);
            let mut alive = claimed();
            for _ in 0..20 {
                if alive {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
                alive = claimed();
            }
            if alive {
                return Err(Error::Sandbox(format!("pod '{name}' already exists")));
            }
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir)
                .map_err(|e| Error::Sandbox(format!("pod dir: {e}")))?;
        } else {
            return Err(Error::Sandbox(format!("pod dir: {e}")));
        }
    }
    // Mark this claim as in-progress with OUR `pid:starttime`, BEFORE the (slow) holder startup, so a
    // concurrent loser above sees a live starter and backs off instead of reclaiming a half-built pod.
    // The start-time pins the pid's identity (as the registry does for supervisors) so a stale marker
    // whose pid was later reused by an unrelated process can't wedge `pod create` shut.
    let me = unsafe { libc::getpid() };
    let _ = std::fs::write(
        dir.join("starting"),
        format!("{me}:{}", crate::registry::proc_starttime(me)),
    );
    // Seed the shared /etc/hosts. Every member box bind-mounts this; members are appended on join.
    std::fs::write(
        hosts_path(name),
        "127.0.0.1\tlocalhost\n::1\tlocalhost ip6-localhost\n",
    )
    .map_err(|e| Error::Sandbox(format!("pod hosts: {e}")))?;

    // Spawn the holder: a detached `kern __pod-holder` that unshares + holds the pod user+net ns and
    // prints `pod-ready` once its namespaces are set up. We read that line, then record its PID.
    let self_exe = std::env::current_exe()
        .map_err(|e| Error::Sandbox(format!("cannot locate the kern binary: {e}")))?;
    let mut cmd = std::process::Command::new(self_exe);
    cmd.arg("__pod-holder")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .process_group(0); // its own session/group so it survives this command exiting
    if uid_range.is_on() {
        // Tell the holder to map a subordinate uid range (so member OCI images can drop privilege),
        // and WHY, so it only reports an unavailable range the caller actually asked for.
        cmd.env("KERN_POD_UID_RANGE", uid_range.as_env());
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Sandbox(format!("pod holder: {e}")))?;
    let pid = child.id() as i32;
    // Wait for the holder's `pod-ready` line, but BOUNDED: a holder that wedges during namespace setup
    // (a host/kernel quirk) must NOT hang `pod create` - and with it the whole `compose up` - forever.
    // A reader thread does the blocking `read_line`; if no answer arrives within the timeout we treat
    // the holder as failed, kill it, and error, instead of an unbounded wait on a child's readiness.
    let ready = child.stdout.take().is_some_and(|mut out| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            std::io::BufReader::new(&mut out).read_line(&mut line).ok();
            let _ = tx.send(line.trim() == "pod-ready"); // receiver may be gone on timeout - ignore
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap_or(false)
    });
    if !ready {
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&dir); // also drops the `starting` marker
        return Err(Error::Sandbox(
            "pod holder failed to start (unprivileged user namespaces may be unavailable)".into(),
        ));
    }
    // Record the holder PID + its net ns inode (identity, to reject a later PID reuse). Write the
    // inode FIRST so a concurrent lookup never sees a holder pid without its verifier.
    if let Some(ino) = ns_inode(pid, "net") {
        let _ = std::fs::write(dir.join("netns"), ino.to_string());
    }
    std::fs::write(dir.join("holder"), pid.to_string())
        .map_err(|e| Error::Sandbox(format!("pod holder pid: {e}")))?;
    let _ = std::fs::remove_file(dir.join("starting")); // claim complete: holder pid is now recorded
                                                        // The holder is detached (own process group, reparented to init on our exit) and runs until
                                                        // `kern pod rm`. `forget` just drops our `Child` handle without any wait/kill (std never reaps or
                                                        // signals on drop) - the stdout pipe was already `.take()`n, so nothing leaks.
    std::mem::forget(child);
    // OUTBOUND (default, opt-out with `--no-outbound`): if `pasta` (passt) is installed, attach it to
    // the pod net ns for NAT'd internet egress + DNS. Best-effort: absent/failed → loopback-only.
    // `None` means "not attempted" (`--no-outbound`), and each `Some` is a real outcome. The
    // alternative was a dummy `Outbound` value standing in for "not consulted", which is a lie the
    // type would then carry to every reader of the match below.
    let outbound = want_outbound.then(|| setup_outbound(name, pid));
    println!("created pod '{name}'");
    println!(
        "  add boxes: kern box <name> --pod {name} -d -- …  (publish a service with -p on its box)"
    );
    // One line per CAUSE. This was one line for five different states, and the one it printed told
    // a user with pasta installed to install pasta.
    match outbound {
        None => {
            println!("  network: loopback-only (--no-outbound) - services reach each other; no egress")
        }
        Some(Outbound::Up) => {
            println!("  network: services reach each other by name + outbound to the internet (pasta)")
        }
        Some(Outbound::NotInstalled) => println!(
            "  network: loopback-only - services reach each other; NO outbound (install `passt`/`pasta` for egress)"
        ),
        Some(Outbound::Failed(why)) => println!(
            "  network: loopback-only - services reach each other; pasta IS installed but did not \
             start: {why}"
        ),
        Some(Outbound::NoDns) => println!(
            "  network: outbound is up but DNS is not - the pod can reach an IP and cannot resolve \
             a name (kern could not write the pod's resolv.conf)"
        ),
    }
    Ok(())
}

/// Attach `pasta` (passt) to the pod's net ns for NAT'd outbound + DNS, and seed the pod's
/// `resolv.conf`. Returns `true` only when outbound AND DNS are actually up (so `create`'s message
/// is honest). Best-effort: no pasta / any failure → `false` (the pod stays loopback-only). pasta
/// backgrounds itself and exits automatically when the net ns is freed (at `pod rm`).
/// pasta's automatic port mapping, OFF in all four directions.
///
/// pasta defaults every one of these to `auto`, which is not merely redundant with kern's own
/// publishing - it breaks it:
///
///  * `-t auto` (host → ns) re-scans host-bound ports on a timer and binds the matching port INSIDE
///    the pod net ns. kern's forwarder binds the HOST side of every `-p`, so ~1-2 s after a box
///    starts pasta claims that port in the ns and the service trying to listen there gets EADDRINUSE.
///    Measured on the shipped binary: a service binding within ~1 s won the race, one binding at >=2 s
///    always lost - i.e. every real app (node, django, postgres) failed to serve its own published
///    port while `compose up` still reported success. A silent partial failure, at runtime.
///  * `-T auto` (ns → host) would publish in-pod ports on the host with no `-p` at all, contradicting
///    kern's explicit-publish, loopback-default model. Off by construction rather than by timing.
///
/// Publishing stays entirely kern's job ([`crate::ports`] → `fork_forwarders`: bind the host port,
/// `setns` into the box per connection). pasta is left doing exactly what it is here for: NAT'd
/// egress + DNS.
const PASTA_NO_PORT_MAP: [&str; 8] = ["-t", "none", "-u", "none", "-T", "none", "-U", "none"];

/// The exact argv passed to `pasta` for a pod, as a pure function of the pod dir + holder PID, so the
/// invariants above are unit-testable without spawning anything. `--config-net` copies the host's
/// addresses/routes into the ns tap and NATs outbound; pasta then daemonizes (the spawned process
/// exits once setup is done). `-q` quiets it, `-P` records its PID for teardown.
///
/// `watch_netns == false` adds `--no-netns-quit`, which is the SECOND attempt and never the first.
/// STRACED, not assumed: with the watch on, pasta opens the netns file, the userns file, AND the
/// DIRECTORY that holds them, in that order, immediately before it touches `/dev/net/tun`:
///
///   openat("/proc/<holder>/ns/user", O_RDONLY) = 6
///   openat("/proc/<holder>/ns/net",  O_RDONLY) = 6
///   openat("/proc/<holder>/ns",      O_RDONLY) = 16   <- only for the quit watch
///   openat("/dev/net/tun",           O_RDWR)   = 18
///
/// With `--no-netns-quit` the third line is gone and the fourth takes its place at the same point in
/// the sequence. That open is what a Fedora 43 host under Lima refused with `netns dir open:
/// Permission denied, exiting`, leaving the pod loopback-only (#6).
///
/// "AND NOTHING ELSE" WAS WRONG, and it said so here until an external reviewer asked for the trace
/// that was never taken: the first pass filtered on `openat`, so it could only ever have found an
/// open. Diffing the FULL syscall set of both runs, the flag removes four things, not one:
///
///   openat("/proc/<holder>/ns", …)    the directory the watch would monitor
///   fstatfs                            whether that filesystem can be watched at all
///   timerfd_create + timerfd_settime   the polling fallback when it cannot
///
/// All four are the watch and nothing but the watch, so the claim the retry rests on still holds.
/// The claim that was made is not the claim that was measured, which is the difference this note
/// exists to record. (No `inotify_*` in either run on this host: passt took the timer fallback.)
///
/// WHAT THE FLAG COSTS. pasta no longer exits by itself when the netns disappears, so `teardown`
/// becomes the only thing that reaps it. That is why the pasta kill there is no longer conditional
/// on the holder still being alive: with the watch off, a holder that dies outside teardown would
/// otherwise leave pasta running forever.
fn pasta_args(dir: &std::path::Path, holder: i32, watch_netns: bool) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = Vec::with_capacity(16);
    a.push("--config-net".into());
    a.push("-q".into());
    a.push("-P".into());
    a.push(dir.join("pasta.pid").into());
    a.extend(PASTA_NO_PORT_MAP.iter().map(Into::into));
    if !watch_netns {
        a.push("--no-netns-quit".into());
    }
    a.push("--userns".into());
    a.push(format!("/proc/{holder}/ns/user").into());
    a.push("--netns".into());
    a.push(format!("/proc/{holder}/ns/net").into());
    a
}

/// Does this pasta stderr name the netns-directory open, the one thing `--no-netns-quit` removes?
///
/// NARROW ON PURPOSE. Retrying on any failure would hide the real ones behind a second attempt that
/// changes an unrelated variable; this matches pasta's own string for the single operation the flag
/// elides, so a pod that fails for any other reason still fails once, loudly, with its own message.
///
/// AN EXTERNAL REVIEWER READ THE OTHER PERMISSION STRINGS OUT OF THE BINARY and asked whether these
/// should retry too:
///
///   Couldn't open network namespace %s: %s
///   Couldn't open user namespace %s: %s
///   setns() failed entering netns: %s
///
/// They should not, and the trace says why rather than the argument: `/proc/<holder>/ns/net` and
/// `/proc/<holder>/ns/user` are opened in BOTH runs, with the watch and without it, and `setns` is
/// not part of the watch at all. A host that refuses those refuses them identically on the retry, so
/// widening the match would spend a second attempt that cannot succeed and would bury the real
/// reason under a second copy of itself. The narrowness is the measurement, not caution.
fn is_netns_dir_denial(stderr: &str) -> bool {
    stderr.contains("netns dir open")
}

/// pasta's stderr as ONE reportable sentence.
///
/// Scrubbed like every other borrowed string kern prints: pasta is a local binary and not the threat
/// model that `crate::ui::scrub` was written for, but the filter is free and the alternative is one
/// unscrubbed path that the next reader has to reason about.
///
/// EVERY line, not the first. pasta's message is the whole diagnosis and taking one line of it is
/// wrong whenever the first line is not the error. Measured on WSL2, where pasta prints five lines
/// and the first is informational:
///
///   Started as root, will change to nobody.        <- what kern used to report
///   No interfaces with usable IPv6 routes
///   Couldn't pick external interface: disabling IPv6
///   Could not open /proc/self/uid_map: Permission denied   <- the actual cause
///   Couldn't configure user mappings
///
/// A reader given only the first line is told something true and useless, and would go looking at
/// privilege dropping instead of uid maps. Joined with "; " and capped, so a pasta that decides to
/// be verbose cannot flood the line either.
fn pasta_reason(stderr: &[u8]) -> String {
    let joined = String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    crate::ui::scrub(if joined.is_empty() {
        "no output"
    } else {
        &joined
    })
    .chars()
    .take(300)
    .collect::<String>()
}

/// Why a pod did or did not get outbound. One `bool` used to cover all of these, and `create`
/// printed the same "install `passt`/`pasta` for egress" line for every one of them - including to
/// someone who had `pasta` installed and whose real problem was that it refused to start. Measured
/// on WSL2 with `/usr/bin/pasta` present: the pod came up loopback-only and was told to install the
/// thing it already had, while pasta's own explanation went to `/dev/null`.
enum Outbound {
    /// NAT and DNS are both up.
    Up,
    /// No `pasta` on PATH: the one case the old message actually described.
    NotInstalled,
    /// `pasta` is installed and did not start. Carries its first line of stderr when it produced
    /// one, because that line is the only thing here that says WHY.
    Failed(String),
    /// The NAT attached but the pod's `resolv.conf` could not be written: addresses work, names do
    /// not. Reporting this as "no outbound" was wrong in the other direction.
    NoDns,
}

fn setup_outbound(name: &str, holder: i32) -> Outbound {
    let Some(pasta) = which_pasta() else {
        return Outbound::NotInstalled;
    };
    let dir = pod_dir(name);
    // stderr is CAPTURED, not discarded: when pasta refuses, its message is the whole diagnosis.
    let spawn = |watch_netns: bool| {
        std::process::Command::new(&pasta)
            .args(pasta_args(&dir, holder, watch_netns))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
    };
    match spawn(true) {
        Ok(o) if o.status.success() => {}
        Ok(o) if is_netns_dir_denial(&String::from_utf8_lossy(&o.stderr)) => {
            // ONE narrower retry, and only for this refusal. pasta opens the netns's DIRECTORY
            // solely to watch it and quit when it disappears; `--no-netns-quit` drops that open and
            // nothing else (straced, see `pasta_args`). A host whose policy refuses that open still
            // permits the netns and userns FILES, so the NAT itself is reachable without the watch.
            //
            // Attempted rather than assumed to be the whole story: if the second attempt also fails,
            // BOTH reasons are reported. Reporting only the second would hide the first, and the
            // first is the one that names the operation a policy refused.
            let first = pasta_reason(&o.stderr);
            match spawn(false) {
                Ok(o2) if o2.status.success() => {}
                Ok(o2) => {
                    return Outbound::Failed(format!(
                        "{first}; retried without the netns watch and it also failed: {}",
                        pasta_reason(&o2.stderr)
                    ));
                }
                Err(e) => {
                    return Outbound::Failed(format!(
                        "{first}; the retry without the netns watch could not be spawned: {e}"
                    ));
                }
            }
        }
        Ok(o) => return Outbound::Failed(pasta_reason(&o.stderr)),
        Err(e) => return Outbound::Failed(e.to_string()),
    }
    // Seed the pod resolv.conf with the host's real (non-loopback) nameservers - reachable through
    // the NAT, so split-horizon/LAN DNS keeps working. Only if the host has NONE that are usable from
    // the ns (e.g. systemd-resolved's 127.0.0.53 stub) do we fall back to a public resolver.
    let mut resolv = String::new();
    if let Ok(host) = std::fs::read_to_string("/etc/resolv.conf") {
        for l in host.lines() {
            if let Some(ns) = l.strip_prefix("nameserver ") {
                // A resolv.conf value is a single token; take it and drop any trailing comment.
                let ns = ns.split_whitespace().next().unwrap_or("");
                if !ns.starts_with("127.") && !ns.is_empty() {
                    resolv.push_str(&format!("nameserver {ns}\n"));
                }
            }
        }
    }
    if resolv.is_empty() {
        resolv.push_str("nameserver 1.1.1.1\n"); // host has only a local stub → public fallback
    }
    // DNS is only "up" if we actually wrote the resolv.conf the box will bind - else don't claim it.
    // Distinguished from "no outbound at all": the NAT is attached either way, so a box can reach an
    // IP but not resolve a name, and saying "no outbound" would send the reader after the wrong
    // thing entirely.
    if std::fs::write(resolv_path(name), resolv).is_ok() {
        Outbound::Up
    } else {
        Outbound::NoDns
    }
}

/// Locate the `pasta` binary (part of passt), or `None` if it isn't installed.
fn which_pasta() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("pasta"))
            .find(|p| p.is_file())
    })
}

/// Append a member to a pod's shared `/etc/hosts` (name → `127.0.0.1`) if not already present, so
/// every member box can resolve it. Idempotent.
pub fn add_member(name: &str, member: &str) -> Result<(), Error> {
    let hp = hosts_path(name);
    let body = std::fs::read_to_string(&hp).unwrap_or_default();
    let line = format!("127.0.0.1\t{member}\n");
    if body.lines().any(|l| {
        let mut it = l.split_whitespace();
        it.next() == Some("127.0.0.1") && it.next() == Some(member)
    }) {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&hp)
        .map_err(|e| Error::Sandbox(format!("pod hosts: {e}")))?;
    f.write_all(line.as_bytes())
        .map_err(|e| Error::Sandbox(format!("pod hosts: {e}")))?;
    Ok(())
}

/// `kern pod ls` - list pods (name, member count, alive/dead holder).
/// The pods on disk, sorted, as (name, member count, holder alive).
///
/// Split out of [`list`] so the human table and `--json` read the SAME scan. Two scanners is how
/// `kern validate` and `kern config list` once gave two verdicts about one file, and a pod that
/// shows `up` in the table and `"alive": false` in JSON would be the same defect with a worse
/// blast radius, because only one of the two is what a script acts on.
fn rows() -> Vec<(String, usize, bool)> {
    let root = pods_root();
    // MEMBERS COME FROM THE REGISTRY, NOT FROM THE SHARED `hosts` FILE. Counting hosts lines beyond
    // the two localhost seeds was right until aliases existed: `kern box --pod` adds ONE entry (the
    // box name, `add_member` in `start.rs`) while a compose service adds TWO (the qualified
    // `<pod>-<service>` and the bare alias, `add_member` in `compose.rs`), so this and `--json` both
    // reported exactly DOUBLE for every compose stack. MEASURED on the shipped v0.9.1 binary: 1, 2
    // and 3 services read 2, 4 and 6 here while `kern ps` read 1, 2 and 3.
    //
    // The comment on this function says the human table and `--json` were unified so they cannot
    // disagree. They could not, and both were wrong: a THIRD view (`kern ps`, which filters
    // `registry::list()` on `pod`) had the right answer and was never reconciled with them. Unifying
    // two readers is not the same as reading the right thing, so this now reads what `ps` reads.
    //
    // Scanned ONCE, outside the loop: `list()` walks the registry dir, and doing it per-pod would
    // make `pod ls` O(pods x boxes) for a number both views already have.
    let live = crate::registry::list();
    let mut rows: Vec<(String, usize, bool)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let alive = holder_pid(&name).is_some();
            let members = live.iter().filter(|i| i.pod == name).count();
            rows.push((name, members, alive));
        }
    }
    rows.sort();
    rows
}

/// `kern pod ls --json`: one array, one line, empty when there are no pods.
///
/// `[]` and not the human "no pods - create one with ..." line: a script that has to special-case a
/// sentence to learn there is nothing there is parsing prose. The name is escaped because it is a
/// directory name on disk.
pub fn list_json() -> Result<(), Error> {
    let out = kern_common::json_array(&rows(), |(name, members, alive)| {
        format!(
            "{{\"name\":{},\"boxes\":{},\"alive\":{}}}",
            kern_common::json_str(name),
            members,
            alive,
        )
    });
    println!("{out}");
    Ok(())
}

pub fn list() -> Result<(), Error> {
    let rows = rows();
    if rows.is_empty() {
        println!("no pods - create one with `kern pod create <name>`");
        return Ok(());
    }
    let p = crate::ui::Palette::detect();
    println!(
        "{d}{:<24} {:>7}  STATUS{z}",
        "POD",
        "BOXES",
        d = p.d,
        z = p.z
    );
    for (name, members, alive) in &rows {
        let status = if *alive { "up" } else { "dead" };
        println!("{}{}{:<24}{} {:>7}  {status}", p.b, p.c, name, p.z, members);
    }
    Ok(())
}

/// Does this `/proc/<pid>/comm` belong to the `pasta`/`passt` family? passt re-execs into an
/// ISA-optimized variant, so `comm` is `pasta.avx2` (or `passt.avx512`, …) - never the bare `pasta`.
/// Matching by family prefix is what keeps the teardown's PID-reuse check from silently leaking the
/// NAT daemon (the bug where `comm == "pasta"` never matched → pasta survived every `pod rm`).
///
/// THE FAMILY IS `<base>` OR `<base>.<variant>`, not "anything starting with pasta". A bare
/// `starts_with` also accepts `pastafarian`, which an external reviewer pointed out, and the
/// teardown now signals a recorded pid unconditionally rather than only while the holder lives, so
/// the guard carries more weight than it did. The variant is always introduced by a `.`, so
/// requiring that separator costs one comparison and removes the whole class of unrelated names
/// that merely share a prefix.
fn is_pasta_comm(comm: &str) -> bool {
    ["pasta", "passt"]
        .iter()
        .any(|base| comm == *base || comm.strip_prefix(base).is_some_and(|r| r.starts_with('.')))
}

/// Tear a pod down: kill its pasta NAT daemon (verified by PID + `comm` family prefix), then its holder, then wipe
/// its state dir. Returns `(existed, member_count)`. Silent - callers do the messaging so `pod rm`
/// and `compose down` can each say the right thing. Member boxes keep their own (already-joined)
/// namespaces until they exit; only the holder is freed.
pub fn teardown(name: &str) -> (bool, usize) {
    // Validate BEFORE building the path: `pod_dir` is `pods/<name>`, and an unvalidated `name` like
    // `../../x` would make `remove_dir_all` escape the pod store and wipe an unrelated directory. Only
    // `create` validated before; `pod rm` / `compose down` reach here with raw input, so guard here
    // too (all callers). An invalid name simply matches no pod.
    if validate_name(name).is_err() {
        return (false, 0);
    }
    let dir = pod_dir(name);
    if !dir.is_dir() {
        return (false, 0);
    }
    // Members from the REGISTRY, and BEFORE anything is killed. Same defect and same fix as `rows()`
    // above: a compose member writes TWO `hosts` lines (qualified name + alias) and a `kern box
    // --pod` member writes one, so the old shared-hosts count doubled for a compose stack and was
    // right for a hand-made pod. Read first because `list()` prunes dead entries as it scans: taking
    // it after the holder dies would race the members that exit with it and report a low number for
    // the same teardown that a caller is about to print.
    let members = crate::registry::list()
        .iter()
        .filter(|i| i.pod == name)
        .count();
    // Kill pasta FIRST, while the holder still owns the net ns - so its recorded PID is unambiguously
    // pasta (killing the holder frees the ns → pasta auto-exits → PID-reuse window). Verify via comm
    // (pasta runs in the HOST net ns, so the holder's ns-inode guard can't cover it).
    //
    // UNCONDITIONAL, and it was not. This used to run only `if holder.is_some()`, on the assumption
    // that a dead holder means pasta already noticed the netns vanish and left. That assumption is
    // no longer safe: a pod whose pasta was started with `--no-netns-quit` (the retry in
    // `setup_outbound`, for hosts that refuse the netns-dir open) does NOT watch the namespace and
    // does not exit on its own, so gating on a live holder would leak it for the life of the
    // session. The `comm` check below is what makes killing safe when the holder is already gone: it
    // is the guard against the recycled PID that the old gate was standing in for.
    // `holder_to_reap`, not `holder_pid`: the second says whether a box may join this namespace, and
    // its `None` also covers "could not tell", which is exactly the case that used to leak a live
    // holder one line before the directory naming it was deleted.
    let holder = holder_to_reap(name);
    if let Ok(pp) = std::fs::read_to_string(dir.join("pasta.pid")) {
        if let Ok(pp) = pp.trim().parse::<i32>() {
            // `kill(0, ...)` signals the caller's own process group and `kill(-1, ...)` signals
            // every process it may signal; a degenerate value in that file must never reach `kill`.
            if pp > 0 {
                let is_pasta = std::fs::read_to_string(format!("/proc/{pp}/comm"))
                    .map(|c| is_pasta_comm(c.trim()))
                    .unwrap_or(false);
                if is_pasta {
                    unsafe { libc::kill(pp, libc::SIGTERM) };
                }
            }
        }
    }
    if let Some(pid) = holder {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    let _ = std::fs::remove_dir_all(&dir);
    (true, members)
}

/// `kern pod rm <name>` - tear the pod down; still-running member boxes keep going until they exit.
pub fn remove(names: &[String]) -> Result<(), Error> {
    if names.is_empty() {
        return Err(Error::Usage("pod rm <name>..."));
    }
    let mut missing = Vec::new();
    for name in names {
        let (existed, members) = teardown(name);
        if !existed {
            missing.push(name.clone());
            eprintln!("kern: no pod named '{name}'");
            continue;
        }
        println!("removed pod '{name}'");
        if members > 0 {
            println!("  ({members} member box(es) keep running until they exit; `kern stop` them)");
        }
    }
    // A `pod rm <name>` that removed NOTHING (every name was missing) must exit non-zero, so a script
    // can tell the removal failed - parity with `config rm` / `volume rm` on an unknown name.
    if missing.len() == names.len() {
        return Err(Error::NotRunning(format!(
            "no pod named '{}'",
            missing.join("', '")
        )));
    }
    Ok(())
}

/// `kern __pod-holder` (hidden): become the pod's namespace holder - never returns.
pub fn run_holder() -> ! {
    kern_isolation::run_pod_holder()
}

/// Pod names share the box-name charset (used as a directory + hostnames): `[A-Za-z0-9_.-]`, ≤64,
/// no traversal. Rejects anything that could escape `pods/` or corrupt `/etc/hosts`.
fn validate_name(name: &str) -> Result<(), Error> {
    // The shared [`kern_common::valid_resource_name`] rule (one definition for volumes/secrets/pods/
    // profiles): also rejects a leading `-` and any `..` substring, which the old local rule missed.
    if kern_common::valid_resource_name(name) {
        Ok(())
    } else {
        Err(Error::Sandbox(format!(
            "invalid pod name '{name}' (use letters, digits, '_', '-', '.'; no leading '-'/'.' or '..'; max 64)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_names_reject_traversal_and_bad_chars() {
        for ok in ["web", "my-app", "db_1", "v1.2"] {
            assert!(validate_name(ok).is_ok(), "{ok} should be valid");
        }
        for bad in [
            "",
            "../evil",
            "a/b",
            ".hidden",
            "has space",
            &"x".repeat(65),
        ] {
            assert!(validate_name(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn network_sentence_does_not_blame_a_refusal_that_never_happened() {
        // THE ARM THAT WAS MISSING, found by an external reviewer reading the four arms rather
        // than running anything. `resolv.conf` is written only after pasta has already started, so
        // "pasta is not alive AND its resolv.conf exists" means it came up and later died: crashed,
        // OOM-killed, or caught by a racing teardown. It used to fall into the "installed but not
        // running" arm, which tells the reader `pod create` explains why it refused. Nothing
        // refused, create succeeded, and no such line was ever printed, so the message sent the
        // reader after an explanation that does not exist. Same shape as #6's symptom, one arm
        // over, in the function written to fix #6.
        let died = network_sentence(false, true, true);
        assert!(
            !died.contains("refused"),
            "a pasta that started and died was never refused: {died}"
        );
        assert!(
            !died.contains("install"),
            "it is installed, and #6 was exactly this wrong remedy: {died}"
        );
        assert!(died.contains("has since exited"), "{died}");
        // `installed` must not change that verdict: the resolv.conf already settled it.
        assert_eq!(died, network_sentence(false, true, false));

        // The refusal arm keeps its sentence, and only for the state that produced it.
        let refused = network_sentence(false, false, true);
        assert!(refused.contains("says why it refused"), "{refused}");
        assert!(!refused.contains("install `passt`"), "{refused}");

        // Genuinely absent pasta is the only arm that may say "install".
        let absent = network_sentence(false, false, false);
        assert!(absent.contains("install `passt`"), "{absent}");

        // The two healthy arms are distinct, and only the fully-healthy one claims the internet.
        let up = network_sentence(true, true, false);
        let nodns = network_sentence(true, false, false);
        assert!(up.contains("outbound to the internet"), "{up}");
        assert!(nodns.contains("cannot resolve"), "{nodns}");
        assert_ne!(up, nodns);

        // All five reachable states say five different things: the collapse into two is what
        // shipped as #6, so distinctness is the property under test, not the wording.
        let all = [died, refused, absent, up, nodns];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "two states share a sentence");
            }
        }
    }

    #[test]
    fn pasta_comm_matches_isa_variants_not_strangers() {
        // Regression: the teardown once compared `comm == "pasta"` and never matched the real
        // `pasta.avx2`, so the NAT daemon leaked on every `pod rm` / `compose down`.
        for ok in [
            "pasta",
            "passt",
            "pasta.avx2",
            "passt.avx512",
            "pasta.avx2\n".trim(),
        ] {
            assert!(is_pasta_comm(ok), "{ok} should match the pasta family");
        }
        // A shared PREFIX is not membership of the family, and the teardown now signals a recorded
        // pid whether or not the holder is still alive, so a name that merely starts with `pasta`
        // must not be enough. `pastafarian` was named by an external reviewer; the rest are the same
        // shape. The variant separator is always `.`, so anything else after the base is a stranger.
        for no in [
            "bash",
            "sleep",
            "kern",
            "past",
            "asta",
            "",
            "pas",
            "pastafarian",
            "passthrough",
            "pasta_helper",
            "pasta-avx2",
            "pastad",
        ] {
            assert!(!is_pasta_comm(no), "{no} must NOT match");
        }
    }

    #[test]
    fn pasta_argv_disables_automatic_port_mapping() {
        // REGRESSION GUARD for a runtime silent partial failure: pasta defaults `-t/-u/-T/-U` to
        // `auto`, and `-t auto` periodically binds host-bound ports INSIDE the pod net ns. Since kern's
        // own forwarder binds the host side of every `-p`, pasta would steal the published port from
        // the service ~1-2 s after start (measured: bind at >=2 s always got EADDRINUSE) while
        // `compose up` still reported success. All four MUST stay explicitly `none`.
        // BOTH modes, because the retry path in `setup_outbound` builds this argv too and a guard
        // that covers only the first attempt stops covering the run that a Fedora host actually gets.
        for watch_netns in [true, false] {
            let argv = pasta_args(
                std::path::Path::new("/run/user/1000/kern/pods/demo"),
                4242,
                watch_netns,
            );
            let flat: Vec<String> = argv
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            for dir_flag in ["-t", "-u", "-T", "-U"] {
                let at = flat.iter().position(|a| a == dir_flag);
                let Some(at) = at else {
                    panic!("pasta argv must pin {dir_flag} explicitly, got {flat:?}");
                };
                assert_eq!(
                    flat.get(at + 1).map(String::as_str),
                    Some("none"),
                    "{dir_flag} must be 'none' (pasta's default is 'auto', which steals published \
                     ports); watch_netns={watch_netns}"
                );
            }
            // The rest of the contract the teardown and egress depend on.
            assert!(flat.contains(&"--config-net".to_string()), "NAT'd egress");
            let pidfile = flat
                .iter()
                .position(|a| a == "-P")
                .and_then(|i| flat.get(i + 1));
            assert_eq!(
                pidfile.map(String::as_str),
                Some("/run/user/1000/kern/pods/demo/pasta.pid"),
                "teardown reads this exact path to kill pasta"
            );
            let ns = flat
                .iter()
                .position(|a| a == "--netns")
                .and_then(|i| flat.get(i + 1));
            assert_eq!(ns.map(String::as_str), Some("/proc/4242/ns/net"));
            let us = flat
                .iter()
                .position(|a| a == "--userns")
                .and_then(|i| flat.get(i + 1));
            assert_eq!(us.map(String::as_str), Some("/proc/4242/ns/user"));
        }
    }

    /// The holder is identified by argv POSITION, because the answer decides a `SIGKILL`.
    ///
    /// `teardown` reaps a live holder whose netns inode could not be checked, which is the case that
    /// leaked one process per pod; this is what says the process is kern's own. The first version
    /// asked only whether ANY argument equalled the marker, and this test is what showed that
    /// `kern box x -- echo __pod-holder` satisfies it: a whole argument, belonging to the workload.
    #[test]
    fn holder_is_identified_by_argv_position_not_by_presence() {
        let cmd = |args: &[&str]| {
            let mut v = Vec::new();
            for a in args {
                v.extend_from_slice(a.as_bytes());
                v.push(0);
            }
            v
        };
        // The real thing, however kern was installed.
        assert!(cmdline_is_holder(&cmd(&[
            "/usr/local/bin/kern",
            "__pod-holder"
        ])));
        assert!(cmdline_is_holder(&cmd(&["kern", "__pod-holder"])));
        assert!(cmdline_is_holder(&cmd(&[
            "./target/debug/kern",
            "__pod-holder"
        ])));
        // A holder carries nothing after the marker today; a future flag must not unmake it one.
        assert!(cmdline_is_holder(&cmd(&["kern", "__pod-holder", "--x"])));

        for argv in [
            // THE ONE THAT BROKE THE FIRST VERSION: the marker is the WORKLOAD's own argument.
            vec!["kern", "box", "x", "--", "echo", "__pod-holder"],
            // argv[1] is right and the program is not kern.
            vec!["grep", "__pod-holder", "/proc/1/cmdline"],
            vec!["/usr/bin/pkill", "__pod-holder"],
            // kern, and any other subcommand.
            vec!["kern", "box", "app"],
            vec!["kern", "ps"],
            vec!["kern"],
            // Substrings, which the token split already rejected and must keep rejecting.
            vec!["kern", "--flag=__pod-holder"],
            vec!["kern", "__pod-holder-ish"],
            vec!["kern", "x__pod-holder"],
            // A binary whose name merely ends with or extends kern.
            vec!["/usr/bin/mykern", "__pod-holder"],
            vec!["kernel", "__pod-holder"],
        ] {
            assert!(
                !cmdline_is_holder(&cmd(&argv)),
                "{argv:?} must not read as the pod holder"
            );
        }
        // Degenerate inputs: empty, only separators, and a buffer with no separator at all.
        assert!(!cmdline_is_holder(b""));
        assert!(!cmdline_is_holder(b"\0\0\0"));
        assert!(!cmdline_is_holder(b"__pod-holder"));
        assert!(!cmdline_is_holder(b"kern"));
    }

    /// `--no-netns-quit` appears on the RETRY and never on the first attempt, and the retry is
    /// entered only for the one refusal it removes.
    ///
    /// The flag has a cost (pasta stops reaping itself when the namespace goes), so a build that
    /// passed it unconditionally would leak a pasta on every host, not only the ones that need it.
    /// The two directions are asserted separately because "present when needed" and "absent
    /// otherwise" are two claims, and the defect that motivated all of this was one condition
    /// standing in for two.
    #[test]
    fn no_netns_quit_is_the_retry_only_and_matches_only_its_own_refusal() {
        let dir = std::path::Path::new("/run/user/1000/kern/pods/demo");
        let has = |watch: bool| {
            pasta_args(dir, 4242, watch)
                .iter()
                .any(|a| a == "--no-netns-quit")
        };
        assert!(
            !has(true),
            "the first attempt must keep pasta's netns watch"
        );
        assert!(has(false), "the retry must drop it, or the refusal recurs");

        // pasta's own string for the open that the flag elides, as reported in #6.
        assert!(is_netns_dir_denial(
            "netns dir open: Permission denied, exiting"
        ));
        // Every other failure must fail once, with its own message, and never be retried behind a
        // second attempt that changed an unrelated variable. These are real pasta stderr lines.
        for other in [
            "Couldn't open user namespace /proc/1/ns/user: Permission denied",
            "Could not open /proc/self/uid_map: Permission denied",
            "TUNSETIFF failed: Device or resource busy",
            "No routable interface for IPv6: IPv6 is disabled",
            "",
        ] {
            assert!(
                !is_netns_dir_denial(other),
                "{other:?} must not trigger the netns-watch retry"
            );
        }
    }

    #[test]
    fn starter_alive_false_for_dead_or_absent() {
        // A cleaned-up temp dir with no `starting` marker → not alive.
        let dir = std::env::temp_dir().join(format!("kern-pod-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert!(!starter_alive(&dir), "no marker → not alive");
        // A marker naming an impossible pid → not alive (kill(pid,0) fails).
        std::fs::write(dir.join("starting"), "2147483646").unwrap();
        assert!(!starter_alive(&dir), "dead/absent pid → not alive");
        // Our own live pid, bare (back-compat marker) → alive.
        let me = std::process::id();
        std::fs::write(dir.join("starting"), me.to_string()).unwrap();
        assert!(starter_alive(&dir), "our live bare pid → alive");
        // `pid:starttime` with the CORRECT start-time → alive (the winner's real marker).
        let st = crate::registry::proc_starttime(me as i32);
        std::fs::write(dir.join("starting"), format!("{me}:{st}")).unwrap();
        assert!(
            starter_alive(&dir),
            "live pid + matching start-time → alive"
        );
        // Same live pid but a WRONG start-time → treated as a reused pid → not a live starter.
        std::fs::write(dir.join("starting"), format!("{me}:{}", st.wrapping_add(1))).unwrap();
        assert!(
            !starter_alive(&dir),
            "start-time mismatch → reused pid → not alive"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
