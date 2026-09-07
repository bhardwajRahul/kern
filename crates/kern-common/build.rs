//! What build is this? `kern --version` used to answer `0.0.0` for every binary that was not cut by
//! the release workflow, which is every binary anyone compiles from source.
//!
//! That is not cosmetic. It cost a measurement in this repo: two binaries of the same tree, one built
//! ten minutes before a fix and one after, were compared and reported as if they were the same
//! program, because neither could say which it was. The external report that pushed this over the line
//! made the same point from the other side: a test script had to print a `sha256sum` to tell one build
//! from another, and that workaround exists in the script only because the binary cannot answer.
//!
//! THE VERSION IS STILL THE TAG. Nothing is carved into the source. Two cases, and neither invents a
//! number:
//!
//!   * `Cargo.toml` says something other than `0.0.0` - the release workflow rewrote it from the tag
//!     it is building. Use it verbatim, so a shipped `kern --version` prints `0.9.3` exactly as before.
//!     This is the ONLY case that reaches a user who downloaded a release.
//!   * `Cargo.toml` still says `0.0.0` - a build from source. Ask git, which knows the truth without
//!     being told: `v0.9.2-45-gf7622ee-dirty` is the nearest tag, the distance from it, the commit,
//!     and whether the tree had uncommitted changes. The `-dirty` suffix is the part that would have
//!     caught the mistake above.
//!
//! Where git cannot answer (a source tarball with no repository, a vendored build), it falls back to
//! `0.0.0`, which is exactly today's behaviour. Nothing regresses when the extra information is
//! unavailable.

use std::process::Command;

fn main() {
    // The build script must re-run when HEAD moves, or the string goes stale the moment you commit and
    // cargo decides nothing changed. Best-effort: a missing git dir simply emits no directive.
    if let Some(git_dir) = run(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/refs");
    }

    let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let version = if cargo_version != "0.0.0" && !cargo_version.is_empty() {
        // Stamped from the tag by the release workflow. Verbatim, and no git call at all: a release is
        // built in a checkout whose describe output we do not want leaking into the shipped string.
        cargo_version
    } else {
        // `--tags` so lightweight tags count, `--always` so a repo with no tag yet still yields the
        // commit rather than nothing, `--dirty` so an uncommitted tree says so.
        run(&["describe", "--tags", "--always", "--dirty"]).unwrap_or(cargo_version)
    };

    println!("cargo:rustc-env=KERN_VERSION={version}");
}

/// Run git and return trimmed stdout, or `None` for any failure at all: git absent, not a repository,
/// a non-zero exit, output that is not UTF-8. A build must not fail because version metadata is
/// unavailable, so every error takes the same quiet path.
fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
