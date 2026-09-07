#!/usr/bin/env python3
"""Does the live getkern.dev page advertise the version that is actually released?

THE SITE IS OUTSIDE THE REPO, so `stale-numbers.py` never sees it: that gate reads `.md` files only,
and the page is HTML on a VPS. MEASURED on 2026-09-07, minutes after v0.9.3 was tagged and its assets
published: the page's JSON-LD still said

    "softwareVersion": "0.9.2"

and nothing in this repository could have noticed. The number is structured data, so it is what a
search engine and an AI answer quote back at a reader, and it had been one release behind before
without anyone catching it. It is the same class as the GPU claim `site-gpu-claims.py` exists for:
a public page saying something about the product that is not true.

WHY IT IS COMPARED AGAINST A TAG AND NOT A CONSTANT. Nothing is carved in here, exactly as
`build.rs` carves nothing: the caller passes the version, and the workflow derives it from the
newest tag in the checkout. A constant in this file would be one more number to forget.

Usage:
    python3 scripts/site-version-current.py <html-file> <expected-version>   # check that page
    python3 scripts/site-version-current.py                                   # self-test, no network
"""

import json
import re
import sys

# The JSON-LD field. Matched narrowly rather than by grepping for the digits: a page mentions plenty
# of numbers, and the one that matters is the structured claim, not prose about benchmarks.
FIELD = re.compile(r'"softwareVersion"\s*:\s*"([^"]+)"')


def declared(html: str) -> list[str]:
    """Every `softwareVersion` the page declares, in order.

    A LIST, not the first match, because two of them disagreeing is its own defect: a page with one
    stale block and one fresh block is worse than a page that is uniformly behind, since whoever
    updates it next will fix the one they can see.
    """
    return FIELD.findall(html)


def check(html: str, expected: str) -> list[str]:
    """Problems with this page, empty when it agrees. `expected` may carry a leading `v`."""
    want = expected.lstrip("v")
    found = declared(html)
    if not found:
        return [
            "the page declares no `softwareVersion` at all: either the JSON-LD block was dropped "
            "or its shape changed, and this check has stopped protecting anything"
        ]
    bad = [v for v in found if v != want]
    if not bad:
        return []
    out = [f"the page says softwareVersion {v!r}, the released version is {want!r}" for v in bad]
    if len(set(found)) > 1:
        out.append(f"the page declares MORE THAN ONE version: {sorted(set(found))}")
    return out


def selftest() -> int:
    """Cases on fixtures, so this file is covered by `gates-selftest.py` without a network call.

    Each case names what it proves. The negative controls matter more than the positive one: a check
    that only ever passes is what the site had before this existed.
    """
    page = '<script type="application/ld+json">{"softwareVersion": "%s"}</script>'
    cases = [
        ("agrees", check(page % "0.9.3", "v0.9.3"), True),
        ("a leading v on the expected version is tolerated", check(page % "0.9.3", "0.9.3"), True),
        ("THE REAL DEFECT: page one release behind", check(page % "0.9.2", "v0.9.3"), False),
        ("a page ahead of the release is refused too", check(page % "0.9.4", "v0.9.3"), False),
        ("no field at all is refused, not passed", check("<html>nothing</html>", "v0.9.3"), False),
        (
            "two disagreeing blocks are refused",
            check(
                '{"softwareVersion": "0.9.3"} {"softwareVersion": "0.9.2"}',
                "v0.9.3",
            ),
            False,
        ),
    ]
    bad = 0
    for name, problems, want_ok in cases:
        ok = not problems
        if ok == want_ok:
            print(f"  ok   {name}")
        else:
            print(f"  FAIL {name}: {problems}")
            bad += 1
    return 1 if bad else 0


def main(argv: list[str]) -> int:
    if len(argv) == 1:
        return selftest()
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[-3], file=sys.stderr)
        return 2
    html = open(argv[1], encoding="utf-8", errors="replace").read()
    problems = check(html, argv[2])
    if not problems:
        print(f"getkern.dev advertises {argv[2].lstrip('v')}, which is the released version")
        return 0
    for p in problems:
        print(f"::error::{p}", file=sys.stderr)
    print(
        "fix it on the server, then re-run:\n"
        "  ssh root@getkern.dev \"sed -i 's/\\\"softwareVersion\\\": \\\"OLD\\\"/"
        '\\"softwareVersion\\": \\"NEW\\"/\' /var/www/getkern.dev/index.html"\n'
        "Cloudflare caches the page for 2 hours, so the served copy lags the file.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
