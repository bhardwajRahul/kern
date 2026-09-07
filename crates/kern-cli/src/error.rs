//! Error type for the CLI.
//!
//! A hand-rolled enum keeps the binary dependency-free. The roadmap target is a
//! `thiserror`-derived enum per crate (see ARCHITECTURE.md); the *shape* - a typed error with
//! an optional actionable hint, mapped to an exit code in one place - is already here.

#[derive(Debug)]
pub enum Error {
    UnknownCommand(String),
    /// A box name failed validation (path separator / traversal / empty).
    InvalidBox(&'static str),
    /// An operational/validation failure inside a box command (a bad `-v` spec, a secret, a `box cp`,
    /// a pod op…). The message is self-explanatory, so it carries no generic hint - unlike [`Setup`],
    /// which is the genuine "the sandbox couldn't start here" failure. ([`Setup`]: Error::Setup)
    Sandbox(String),
    /// The sandbox itself could not be created/run (namespaces, mounts, exec) - an environment
    /// problem, not bad input. Carries the "needs unprivileged user namespaces" hint.
    Setup(String),
    /// A named box isn't running (or has no logs) - a lookup miss, not a setup failure.
    NotRunning(String),
    /// A box name is already held by a live box - a naming conflict, not a setup failure.
    AlreadyRunning(String),
    /// A `kern volume` operation failed for a non-in-use reason (unknown name, bad name, I/O). The
    /// hint points at `kern volume ls` - NOT at boxes (that's [`AlreadyRunning`], used when a volume
    /// is in use) - and is dropped entirely when the message already carries its own remedy, which
    /// it does for the EACCES case. ([`AlreadyRunning`]: Error::AlreadyRunning)
    Volume(String),
    /// An OCI image pull/extract failed.
    Oci(String),
    /// A compose file could not be parsed or brought up.
    Compose(String),
    /// A `kern build` failed: a bad Dockerfile, a COPY that escapes the image, or the build context.
    Build(String),
    /// A `kern.toml` profile could not be parsed, found, or applied.
    Config(String),
    /// A recognised command was invoked with missing/invalid arguments.
    /// A flag or verb the CLI does not take. Displays BARE, with no kind prefix: routed through
    /// `Config` it read `error: config: config list: unknown flag …` with "config" doubled, and through
    /// `Sandbox` it read `error: sandbox: uninstall: unknown flag …`, announcing a parse error as a
    /// sandbox failure. The message already names the verb, so a prefix can only get in the way. Its
    /// hint points at `--help`, unlike `Config`'s, which sent someone who mistyped a flag to read about
    /// where profiles live.
    Cli(String),
    Usage(&'static str),
}

impl Error {
    /// An optional one-line, actionable hint shown under the error.
    pub fn hint(&self) -> Option<String> {
        match self {
            Error::UnknownCommand(_) => Some("run `kern --help` for the list of commands".into()),
            Error::InvalidBox(_) => Some(
                "box names: letters/digits/_/./- only, no leading '-' or '.', max 64 chars".into(),
            ),
            // Operational/validation errors are self-explanatory - no generic hint (it used to
            // wrongly show the userns/rootfs hint on `-v`/secret/port errors).
            Error::Sandbox(_) => None,
            // Branch on the message, for the same reason `oci_hint` does: the setup step that failed
            // decides what the reader should do next, and the variant alone does not know it. An
            // external reviewer measured a box refused by `RLIMIT_NPROC` and got
            //
            //   error: sandbox: fork(idmap helper) failed: Resource temporarily unavailable (os error 11)
            //   hint: needs unprivileged user namespaces and a valid --rootfs directory
            //
            // The message is exact and the hint sends the reader to two places that are both fine.
            // EAGAIN on a fork is a process-limit problem: user namespaces are enabled and the rootfs
            // is valid, or the code would not have reached the fork.
            //
            // Matched on `(os error 11)` rather than on the prose, and the reason is structural
            // rather than empirical: that suffix is APPENDED by `io::Error`'s Display impl, so only
            // the text before it can ever come from libc. It is invariant by construction, which also
            // means a glibc build is as safe as the shipped musl one. Checking locales instead would
            // have proved nothing on the shipped binary, which is static musl with no locale
            // machinery in it at all.
            //
            // The key is asserted from a REAL failure, not from a rendering a test built: see
            // `eagain_hint_survives_a_real_failure_not_a_constructed_one` in tests/smoke.rs. The
            // gap that closes is an errno reaching here through something that is not an
            // `io::Error`, which drops the suffix and reverts this hint in silence.
            // "TASKS (threads), not processes" is not a detail. The reviewer who reported this hint
            // then read `ulimit -u` against a PROCESS count, got 10 against 149, and concluded the
            // kernel was accounting something unobservable. It costs two rounds and a wrong mechanism
            // to omit it, to a reader who already had the errno and a reason to care. Measured here:
            // an x86_64 desktop owned 208 processes and 1918 TASKS, and the limit at which a single
            // fork started succeeding was 1932. Against the task count the threshold IS the count;
            // against the process count it looks like a factor of nine.
            //
            // What is deliberately NOT said: the charge is per-UID across the whole KERNEL, so on a
            // shared-kernel host (WSL2 runs several distributions on one) the tasks are spread over
            // PID namespaces and no single `/proc` can see them all. True, and measured, and it would
            // read as noise to the reader on a laptop. The threads clause is the half that is true
            // everywhere and wrong to omit.
            Error::Setup(msg) if msg.contains("os error 11") => Some(
                "out of process slots: `ulimit -u` is per-UID and counts TASKS (threads), not \
                 processes, across the whole system, so another program owned by this user, or one \
                 with many threads, can exhaust it. Compare `ulimit -u` against the task count, or \
                 raise `LimitNPROC=`/`DefaultLimitNPROC=` for this session"
                    .into(),
            ),
            Error::Setup(_) => {
                Some("needs unprivileged user namespaces and a valid --rootfs directory".into())
            }
            Error::NotRunning(_) => Some("run `kern ps` to see running boxes".into()),
            Error::AlreadyRunning(_) => {
                Some("run `kern ps` to see running boxes; `kern stop <name>` frees the name".into())
            }
            // A volume error that already carries its own remedy needs no generic pointer under it:
            // printing "run `kern volume ls`" beneath two paste-ready commands is noise, and noise
            // under an instruction is how an instruction gets skipped. Multi-line means the message
            // brought its own fix. Same shape as `oci_hint` below, which branches on the message
            // rather than on the variant.
            Error::Volume(msg) => {
                (!msg.contains('\n')).then(|| "run `kern volume ls` to see existing volumes".into())
            }
            // The right hint depends on *why* the pull failed - telling someone whose image name is
            // wrong to "install curl and tar" sends them down the wrong path. Branch on the message.
            Error::Oci(msg) => Some(oci_hint(msg)),
            Error::Compose(_) => {
                Some("compose: `[box.NAME]` tables with image/rootfs, command, depends_on".into())
            }
            // A build-history lookup miss (`build logs|inspect <id>`) is not a Dockerfile problem, so
            // point it at the list - not the FROM/COPY hint, which would mislead. Same message-shape
            // routing as `oci_hint`.
            Error::Build(msg) if msg.starts_with("no build ") => {
                Some("run `kern builds` to list build ids".into())
            }
            Error::Build(_) => Some(
                "build: the Dockerfile must start with FROM (ARG may precede it); COPY/ADD paths \
                 stay inside the image"
                    .into(),
            ),
            Error::Config(_) => {
                Some("profiles live in ~/.config/kern/kern.toml - see docs/CONFIG.md".into())
            }
            Error::Cli(_) | Error::Usage(_) => Some("run `kern --help` for full usage".into()),
        }
    }
}

/// Pick the hint for a pull failure from the shape of its message. `curl`/`tar` missing is a real but
/// *rare* cause; a mistyped name or a private repo is the common one, so only surface the tooling hint
/// when a tool actually failed. The message is the `OciError` Display (or a local cache error).
fn oci_hint(msg: &str) -> String {
    if msg.starts_with("bad image reference") {
        "image refs look like `alpine`, `alpine:3.19`, or `ghcr.io/user/app:tag`".into()
    } else if msg.contains("curl failed")
        || msg.contains("tar failed")
        || msg.contains("sha256sum")
        || msg.contains("zstd")
    {
        "pull/push need `curl`, GNU `tar`, `gzip`, `sha256sum` (and `zstd` for zstd-compressed images) on PATH, plus a working network"
            .into()
    } else if msg.contains("rate-limiting") {
        // The error already says the name and tag are NOT the problem, and already names the way out.
        // Appending the generic name/tag hint under it made the two lines contradict each other, which
        // is worse than no hint: the reader has to guess which half to believe.
        "an authenticated pull has a much higher quota; `kern login <registry>` once and it persists"
            .into()
    } else {
        // Registry / manifest / not-found: the name or tag is the likely culprit.
        "check the image name and tag exist; private images need `kern login` first".into()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownCommand(c) => write!(f, "unknown command '{c}'"),
            Error::InvalidBox(why) => write!(f, "invalid box name: {why}"),
            Error::Sandbox(why) => write!(f, "sandbox: {why}"),
            Error::Setup(why) => write!(f, "sandbox: {why}"),
            Error::NotRunning(why) => write!(f, "{why}"),
            Error::AlreadyRunning(why) => write!(f, "{why}"),
            Error::Volume(why) => write!(f, "{why}"),
            // The OCI error already carries its own kind prefix (`registry:`/`extract:`/`ref:` from
            // `OciError`'s Display), so we don't add another - a doubled "registry: registry:" was the
            // symptom. A local cache error (no OCI prefix) still reads fine on its own.
            Error::Oci(why) => write!(f, "{why}"),
            Error::Compose(why) => write!(f, "compose: {why}"),
            Error::Build(why) => write!(f, "build: {why}"),
            Error::Config(why) => write!(f, "config: {why}"),
            Error::Cli(why) => write!(f, "{why}"),
            Error::Usage(u) => write!(f, "usage: kern {u}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_hint_routes_history_miss_to_the_builds_list() {
        // A build-history lookup miss points at `kern builds`, not the Dockerfile/FROM hint.
        let miss = Error::Build("no build '1-2'".into()).hint().unwrap();
        assert!(miss.contains("kern builds"));
        assert!(!miss.contains("FROM"));
        // A real build error keeps the Dockerfile hint.
        let real = Error::Build("RUN failed (exit 1)".into()).hint().unwrap();
        assert!(real.contains("FROM"));
    }

    /// A setup failure caused by EAGAIN on a fork is a process-limit problem, and the userns/rootfs
    /// hint sends the reader to two places that are both already fine: the code could not have
    /// reached the fork otherwise. Reported by an external reviewer who hit it with a tightened
    /// `ulimit -u`, message exact and hint pointing elsewhere.
    ///
    /// The subject is the REAL rendering, not a hand-written string: the message is built from
    /// `std::io::Error::from_raw_os_error(EAGAIN)` exactly as `Error::last` builds it, so a change in
    /// how Rust renders errno breaks this test rather than the hint in the field.
    #[test]
    fn setup_hint_names_the_process_limit_on_eagain() {
        let rendered = format!(
            "fork(idmap helper) failed: {}",
            std::io::Error::from_raw_os_error(libc::EAGAIN)
        );
        assert!(
            rendered.contains("os error 11"),
            "the match key is gone from the rendering: {rendered}"
        );
        let h = Error::Setup(rendered).hint().unwrap();
        assert!(h.contains("ulimit -u"), "got: {h}");
        assert!(
            !h.contains("user namespaces"),
            "still pointing at userns for a fork that ran out of process slots: {h}"
        );
        // Every other setup failure keeps the hint it had.
        let other = Error::Setup("pivot_root failed: Invalid argument (os error 22)".into())
            .hint()
            .unwrap();
        assert!(other.contains("user namespaces"), "got: {other}");
    }

    #[test]
    fn oci_hint_points_at_the_actual_cause() {
        // A tool failure → the tooling hint.
        assert!(oci_hint("curl failed: exit 6").contains("curl"));
        assert!(oci_hint("tar failed: bad header").contains("tar"));
        // A zstd-compressed image without the `zstd` tool → the tooling hint names zstd.
        assert!(oci_hint(
            "zstd failed: this image uses zstd-compressed layers but `zstd` is not installed"
        )
        .contains("zstd"));
        // A bad reference → the ref-format hint, not tooling.
        let r = oci_hint("bad image reference: alpine::");
        assert!(r.contains("image refs"));
        assert!(!r.contains("curl"));
        // A missing/private image → name/tag/login, not tooling.
        let reg = oci_hint("registry: cannot access 'me/app' - it may be private");
        assert!(reg.contains("kern login"));
        assert!(!reg.contains("curl"));
        // "no manifest for <arch>" and local cache errors fall through to the same safe hint.
        assert!(oci_hint("registry: no manifest for aarch64").contains("image name"));
        // A rate limit must NOT get the name/tag hint: the error itself states that the name is not
        // the problem, and two contradicting lines are worse than one.
        let rl = oci_hint(
            "registry: registry-1.docker.io is rate-limiting this pull of 'library/alpine'",
        );
        assert!(
            !rl.contains("check the image name"),
            "a rate limit must not be hinted as a naming problem: {rl}"
        );
        assert!(
            rl.contains("quota"),
            "the hint must name the actual remedy: {rl}"
        );
    }
}
