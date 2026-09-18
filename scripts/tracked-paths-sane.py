#!/usr/bin/env python3
"""No tracked path may read as a command-line flag, and no rendered guide page may be tracked.

WHY THIS EXISTS, measured. `scripts/build-guide.py` takes its output directory as `argv[1]` and ran
`mkdir` on it without looking at it. Asked for its usage - `build-guide.py --help`, the first thing
anyone types - it created a directory literally called `--help` and wrote the nine rendered site
pages into it. A later `git add -A` committed them, and the repository root on GitHub showed a
folder named after a flag.

Two separate holes, and this closes the second. The script now answers `--help` and refuses any
out-dir starting with `-`; that stops THAT script. This gate stops the class: any tool that takes a
path from an argument can do the same thing, and what they all have in common is where it ends up,
which is `git ls-files`.

WHAT IT REFUSES

  * a path component starting with `-`, which `rm`, `cp` and `tar` read as an option, so removing
    one needs a `--` and a reader has to know that;
  * a rendered guide page, wherever it is tracked. The guide is BUILD OUTPUT: `build-guide.py <dir>`
    regenerates all of it from `docs/*.md`, so a copy in the repository is a second source of truth
    that no gate keeps in step with the first.

It does NOT refuse HTML in general, only a file carrying the generator's own link back to the site.
Measured: none of the paths this repository tracks carries it.

Usage:
    python3 scripts/tracked-paths-sane.py [--self-check]
"""

import pathlib
import subprocess
import sys
import tempfile

# Rendered guide pages carry this, from the generator's own template and sitemap.
GUIDE_MARK = "getkern.dev/guide/"


def tracked() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, text=True, check=True
    ).stdout
    return [p for p in out.split("\0") if p]


def offenders(paths: list[str]) -> list[tuple[str, str]]:
    bad: list[tuple[str, str]] = []
    for path in paths:
        flag = next((p for p in path.split("/") if p.startswith("-")), None)
        if flag is not None:
            bad.append((path, f"the component {flag!r} reads as a command-line flag"))
            continue
        # ANYWHERE, not just the repository root. The accident landed one level down, at
        # `--help/<page>.html`, and a rule tied to a depth would have to guess which depth the next
        # one uses. What identifies the file is its own content.
        if path.endswith((".html", ".xml")):
            try:
                text = pathlib.Path(path).read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            if GUIDE_MARK in text:
                bad.append((path, "rendered guide output; rebuild it with build-guide.py <dir>"))
    return bad


def main() -> int:
    paths = tracked()
    bad = offenders(paths)
    if bad:
        print(f"{len(bad)} tracked path(s) should not be in the repository:", file=sys.stderr)
        for path, why in bad:
            print(f"  {path}\n      {why}", file=sys.stderr)
        print(
            "\nRemove with `git rm -r -- <path>`; the `--` is required for a name starting with -.",
            file=sys.stderr,
        )
        return 1
    print(f"{len(paths)} tracked paths: none reads as a flag, none is rendered guide output")
    return 0


def self_check() -> int:
    """The gate has to be able to FAIL, or a green run proves nothing.

    Both shapes are CONSTRUCTED rather than described, and the clean case is checked too: a gate
    that refuses everything passes the two assertions above while being useless.
    """
    ok = True
    with tempfile.TemporaryDirectory() as d:
        repo = pathlib.Path(d)
        subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
        (repo / "--help").mkdir()
        (repo / "--help" / "x.html").write_text("hi", encoding="utf-8")
        (repo / "stray.html").write_text(f"<a href='{GUIDE_MARK}install.html'>", encoding="utf-8")
        (repo / "deep").mkdir()
        (repo / "deep" / "also.html").write_text(f"see {GUIDE_MARK}faq.html", encoding="utf-8")
        (repo / "fine.md").write_text("ordinary", encoding="utf-8")
        (repo / "page.html").write_text("<p>an ordinary tracked page</p>", encoding="utf-8")
        subprocess.run(["git", "add", "-A"], cwd=repo, check=True)

        r = subprocess.run([sys.executable, __file__], cwd=repo, capture_output=True, text=True)
        for want, label in [
            ("--help/x.html", "a path component that is a flag"),
            ("stray.html", "rendered guide output at the root"),
            ("deep/also.html", "rendered guide output one level down"),
        ]:
            hit = want in r.stderr
            print(f"  {'ok  ' if hit else 'FAIL'} catches {label}")
            ok &= hit
        clean = "page.html\n" not in r.stderr and "fine.md" not in r.stderr
        print(f"  {'ok  ' if clean else 'FAIL'} leaves an ordinary page and an ordinary file alone")
        ok &= clean
        print(f"  {'ok  ' if r.returncode == 1 else 'FAIL'} exits non-zero on a dirty tree")
        ok &= r.returncode == 1

        # `-f`: the files are staged and were never committed, which `git rm` refuses without it.
        subprocess.run(
            ["git", "rm", "-r", "-q", "-f", "--", "--help", "stray.html", "deep"],
            cwd=repo,
            check=True,
        )
        r2 = subprocess.run([sys.executable, __file__], cwd=repo, capture_output=True, text=True)
        print(f"  {'ok  ' if r2.returncode == 0 else 'FAIL'} clears a tree with nothing wrong")
        ok &= r2.returncode == 0

    print("this gate still fails on what it claims to catch" if ok else "THE GATE IS BROKEN")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(self_check() if "--self-check" in sys.argv[1:] else main())
