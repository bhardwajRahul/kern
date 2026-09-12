#!/usr/bin/env python3
"""Every relative link and every anchor in the tracked `.md` files resolves.

WHY THIS EXISTS. A heading is renamed and every link that pointed at it dies in silence: nothing fails,
nothing warns, and the only symptom is a reader landing at the top of a long document instead of at the
paragraph they clicked for. Found twice in one audit on 2026-09-12:

  * `examples/README.md` linked the MCP setup by its OLD title,
    `#use-it-from-claude-desktop-or-cursor-mcp`, after the heading became "Use it from an MCP client
    (Cursor, Claude Desktop, anything that speaks MCP)". The row is the one place the examples index
    sends a reader for the MCP wiring, and it landed them at the top of a 24 KB README.
  * `docs/INSTALL.md` sent its macOS row to `([notes](#install))`, and no heading in that file has that
    slug: the notes are a bold paragraph, not a section.

This is `stale-numbers.py`'s trade applied to navigation: that gate checks a figure a reader could
disprove, this one checks a destination a reader could click.

WHAT IT DOES NOT DO, deliberately: external links. They need the network, they answer 403 to a CI
runner (npmjs.com does, to a browser user agent too, while the registry API says the package is there),
and a gate that fails on someone else's bot filter teaches people to ignore it. External links are
checked by hand, in the same audit that produced this file.

THE SLUG RULE is GitHub's, as far as it goes: lowercase, punctuation dropped, spaces to hyphens. Inline
code and emphasis markers are stripped first, because a heading written with backticks slugs to the
text inside them. Where this and GitHub disagree the answer is to make the link literal rather than to
grow the rule: an anchor a generator cannot predict is one a reader cannot either.

    scripts/md-links.py

Exit 0 iff every relative target exists and every anchor names a heading. The controls run FIRST, so a
checker that resolves everything (or nothing) cannot report a green.
"""

import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LINK = re.compile(r"\[(?:[^\]]*)\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
HEADING = re.compile(r"^#{1,6}\s+(.*?)\s*$")


def slug(text: str) -> str:
    """GitHub's heading slug, for the shapes this repo's headings actually use."""
    t = text.replace("`", "").replace("*", "").replace("_", " ")
    t = re.sub(r"[^\w\- ]", "", t, flags=re.UNICODE).strip().lower()
    return re.sub(r"\s+", "-", t)


def headings(path: str) -> "set[str]":
    out = set()
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            fenced = False
            for line in fh:
                if line.lstrip().startswith(("```", "~~~")):
                    fenced = not fenced
                    continue
                if fenced:
                    continue
                m = HEADING.match(line)
                if m:
                    out.add(slug(m.group(1)))
    except OSError:
        pass
    return out


def tracked_md() -> "list[str]":
    out = subprocess.run(
        ["git", "ls-files", "*.md"], cwd=ROOT, capture_output=True, text=True, check=False
    )
    return [l for l in out.stdout.split("\n") if l.strip()]


def check(files: "list[str]") -> "list[str]":
    """Every broken relative link or anchor, as a printable line."""
    bad = []
    cache: "dict[str, set[str]]" = {}
    for f in files:
        full = os.path.join(ROOT, f)
        try:
            with open(full, encoding="utf-8", errors="replace") as fh:
                text = fh.read()
        except OSError:
            continue
        # A nested image link - `[![alt](img)](target)` - would otherwise be read as a link to `img`.
        text = re.sub(r"!\[[^\]]*\]\([^)]*\)", "IMAGE", text)
        for n, line in enumerate(text.splitlines(), 1):
            for target in LINK.findall(line):
                if target.startswith(("http://", "https://", "mailto:", "#!")):
                    continue
                path, _, anchor = target.partition("#")
                if not path:
                    dest = full
                else:
                    dest = os.path.normpath(os.path.join(os.path.dirname(full), path))
                    if not os.path.exists(dest):
                        bad.append(f"{f}:{n}: no such file: {target}")
                        continue
                if anchor and dest.endswith(".md"):
                    if dest not in cache:
                        cache[dest] = headings(dest)
                    if anchor.lower() not in cache[dest]:
                        where = "this file" if dest == full else os.path.relpath(dest, ROOT)
                        bad.append(f"{f}:{n}: no heading '{anchor}' in {where}: {target}")
    return bad


def main() -> int:
    files = tracked_md()
    if len(files) < 10:
        print(f"md-links: only {len(files)} tracked .md files found, so this gate measured nothing")
        return 2
    print(f"md-links: {len(files)} tracked .md files")

    # CONTROLS FIRST, and BOTH synthetic, so a checker that resolves everything - or nothing - cannot
    # report a green. The first attempt used this script's own file as the positive control and failed
    # instantly: a `.py` whose comments SHOW link syntax is not a markdown document, and the checker
    # read `[![alt](img)](target)` from a comment as a link to a file called `target`. A control has to
    # be made of the thing being measured.
    import tempfile

    def probe_findings(body: str) -> "list[str]":
        with tempfile.NamedTemporaryFile(
            "w", suffix=".md", dir=ROOT, delete=False, encoding="utf-8"
        ) as fh:
            fh.write(body)
            rel = os.path.relpath(fh.name, ROOT)
        try:
            return check([rel])
        finally:
            os.unlink(fh.name)

    good = probe_findings("# A heading\n\n[readme](README.md) and [here](#a-heading)\n")
    if good:
        print(f"  CONTROL FAILED: links that do resolve were reported broken: {good[0]}")
        return 2
    bad_probe = probe_findings("# t\n\n[nope](NO-SUCH-FILE-ANYWHERE.md) and [anchor](#not-a-heading)\n")
    if len(bad_probe) != 2:
        print(
            f"  CONTROL FAILED: a missing file and a missing anchor gave {len(bad_probe)} finding(s), "
            "so this checker cannot see one of them"
        )
        return 2
    print("  controls ok: a real file and a real anchor resolve; a missing one of each is caught")

    bad = check(files)
    for line in bad:
        print(f"  BROKEN {line}")
    if bad:
        print()
        print(f"{len(bad)} link(s) go nowhere. A renamed heading breaks every link to it in silence,")
        print("so fix the link or make the heading the one the link names.")
        return 1
    print("all relative links and anchors resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
