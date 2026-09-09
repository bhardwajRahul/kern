#!/usr/bin/env python3
"""Measure how many compose files kern runs with NO behavioural difference from Docker.

WHY THIS IS A SCRIPT AND NOT A SHELL LOOP SOMEBODY RAN ONCE. The rate has been quoted at 14%, 66%,
93% and 94% during this work, each from an ad-hoc loop that is now gone, so none of them can be
recomputed or argued with. The number is only worth as much as the table below, and the table has to
be readable by someone who wants to disagree with a line of it.

THE MEASUREMENT IS KERN'S OWN WARNINGS, WHICH MAKES IT BLIND BY CONSTRUCTION to any difference kern
does not know it has. That is not a flaw to be worked around, it is the property to keep in view:
the rate can only ever be an upper bound, and it goes DOWN when kern learns about a difference it
was silent on. Measured instance: files carrying `ipv4_address` produced no output at all and scored
as perfect while the literal address is unreachable, which cost two points once it was noticed.

Usage:  compose-compat-rate.py <corpus-dir> [--kern PATH] [--verbose]
"""

import argparse
import pathlib
import re
import subprocess
import sys
from collections import Counter

# A warning that is NOT a behavioural difference from Docker. Every entry states WHY, because the
# whole risk of this table is an inconvenient line being quietly moved into it.
BENIGN = [
    # Docker substitutes an unset variable with the empty string too. Saying so is a courtesy.
    (r"is not set \(no default\) - substituted empty", "Docker substitutes empty as well"),
    (r"empty list item", "the file has a hole in it; kern reports, Docker ignores"),
    # `docker compose up -d` also leaves stdin at EOF: `tty:`/`stdin_open:` describe an attached run.
    (r"a compose service runs detached", "identical under `docker compose up -d`"),
    # `expose:` publishes nothing under Docker either. The line explains, it does not report a loss.
    (r"is DECLARED, not published", "`expose:` publishes nothing under Docker either"),
    # The service is skipped under Docker too, for the same reason.
    (r"not active \(set COMPOSE_PROFILES", "Docker skips an inactive profile identically"),
    # kern already does what the key asks for, so the outcome matches.
    (r"is ALREADY ENFORCED", "kern already does what the key asks"),
    (r"is ENFORCED under --no-pod", "the key is honoured, and this says how"),
    # Announcements of a wiring that REPRODUCES Docker's behaviour rather than departing from it.
    (r"share no network, so they get no relay", "reproduces Docker's segregation"),
    (r"this file separates services with `networks:`", "announces the wiring that matches Docker"),
    (r"puts two services on the same internal port", "announces the wiring that matches Docker"),
    (r"--no-pod gives each service its own network namespace", "announces the chosen wiring"),
    (r"and with a namespace per service kern ENFORCES it", "`internal:` is honoured, and this says how"),
]

# Everything else counts as a difference. Named here only so `--verbose` can group the output; an
# unmatched line is counted as a difference regardless, so a NEW warning is never silently benign.
KNOWN_DIFFERENCES = [
    (r"compose_memory_max", "an operator ceiling caps a service below what the file asks"),
    (r"ONE shared network namespace", "services share 127.0.0.1"),
    (r"mount the Docker socket", "no daemon behind the socket"),
    (r"`ipv4_address` are NOT applied", "the literal address is unreachable"),
    (r"has no kern equivalent \(rootless\)", "`privileged:` cannot be given"),
    (r"NOT applied - the box keeps its own IPC", "`ipc:` not shared"),
    (r"ignored \(unsupported\)", "a key kern does not implement"),
    (r"not honoured - seccomp", "`security_opt` seccomp profile"),
    (r"cannot be honoured - kern runs this machine", "`platform:` mismatch"),
    (r"recognised but not applied", "tmpfs options dropped"),
    (r"long-form", "a volume long form kern cannot express"),
    (r"is not a port in 1", "an out-of-range port is skipped"),
    (r"output is captured", "a `logging:` driver kern cannot provide"),
]

BENIGN_RE = [(re.compile(p), why) for p, why in BENIGN]
KNOWN_RE = [(re.compile(p), why) for p, why in KNOWN_DIFFERENCES]


def differences(text):
    """The lines of one `config` run that are behavioural differences from Docker."""
    out = []
    for line in text.splitlines():
        if not line.startswith("kern:"):
            continue
        if any(rx.search(line) for rx, _ in BENIGN_RE):
            continue
        out.append(line)
    return out


def label(line):
    for rx, why in KNOWN_RE:
        if rx.search(line):
            return why
    # An unrecognised line is still a difference: the table names what is known, it does not decide
    # what counts. A warning added tomorrow lowers the rate until somebody classifies it on purpose.
    return "UNCLASSIFIED (counted as a difference)"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", type=pathlib.Path)
    ap.add_argument("--kern", default="target/release/kern")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    files = sorted(p for p in args.corpus.iterdir() if p.is_file())
    if not files:
        print(f"no compose files under {args.corpus}", file=sys.stderr)
        return 2

    clean, causes, dirty, refused = 0, Counter(), [], []
    for f in files:
        run = subprocess.run(
            [args.kern, "compose", "-f", str(f), "config"],
            capture_output=True,
            text=True,
        )
        # A REFUSED FILE IS NOT A CLEAN ONE, and counting it as clean is how a rate goes UP by
        # rejecting more. The warning scan below sees only `kern: …` lines, so a hard `error:` would
        # otherwise leave a file with no differences at all - a perfect score for a stack that never
        # rendered. Reported apart from the warnings, because the two mean different things: a
        # refusal is either kern agreeing with Docker (`${VAR:?}` with no value) or a file kern
        # cannot read, and only the corpus gate can tell those apart.
        if run.returncode != 0:
            refused.append(f.name)
            continue
        diffs = differences(run.stderr)
        if diffs:
            dirty.append((f.name, diffs))
            for d in diffs:
                causes[label(d)] += 1
        else:
            clean += 1

    total = len(files)
    print(f"corpus              {total} files, one per repository")
    print(f"ZERO differences    {clean} = {clean * 100 // total}%")
    if refused:
        print(f"REFUSED             {len(refused)} (not counted as clean; see compose-corpus-gate.py)")
        for name in refused:
            print(f"                      {name}")
    print("\ncauses, by files affected (a file may have several):")
    for why, n in causes.most_common():
        print(f"  {n:5d}  {why}")
    if args.verbose:
        print("\nfiles with a difference:")
        for name, diffs in dirty:
            print(f"  {name}")
            for d in diffs:
                print(f"      {label(d)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
