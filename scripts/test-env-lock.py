#!/usr/bin/env python3
"""The exemptions from the environment chokepoints are the declared ones, and no others.

WHO CHECKS WHAT, because this file used to try to do all of it and could not:

  * That a test holding no lock cannot read or write a process-global variable is enforced AT
    RUNTIME, by an assertion inside `crate::global_env`, `global_env_str`, `set_global_env` and
    `unset_global_env`. It found 35 tests. A static version of the same rule found 3.
  * That nobody reaches `std::env` around those chokepoints is enforced by CLIPPY, through
    `disallowed-methods` in `clippy.toml`. Clippy resolves the path, so `use std::env;` then
    `env::set_var(..)`, `use std::env::set_var as sv;`, a name held in a binding, and spaces around
    the `::` are all the same call to it. They were four separate holes in the regex that used to
    live here, found by an independent test in one sitting, on the second version of this gate.
  * THIS FILE checks the only thing left: that the list of crates and files exempted from that lint
    is the one written below. An exemption is a whole crate going unwatched, it is one line to add,
    and nothing else would notice.

⭐ The rule the three of them make, which is worth more than this gate: a check that has to PARSE the
language loses to someone who is trying; put the question where the code runs, or ask the compiler.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
# ANY ATTRIBUTE THAT COULD SWITCH THE LINT OFF, not the one spelling we happen to use.
#
# 🪤 The first version of this matched `#![allow(clippy::disallowed_methods)]` exactly, and an
# independent test disarmed the lint three ways it could not see: `expect` instead of `allow`, a
# second lint in the same list, and `allow(clippy::all)`, which never names this lint at all and so
# defeats any regex keyed on its name. The question is not "is this the attribute we write", it is
# "could this attribute turn the lint off": `disallowed_methods` by name, the group that contains it,
# or `warnings` wholesale.
ALLOW = re.compile(
    r"^\s*#!?\[(?:allow|expect)\([^]]*"
    r"(?:disallowed_methods|clippy::all|clippy::style|\bwarnings\b)[^]]*\)\]"
)

# Every place that may reach `std::env` directly, and why. A path missing from here fails this gate.
DECLARED = {
    "crates/kern-cli/src/main.rs": "the chokepoints themselves",
    "crates/kern-cli/tests/sandbox_run.rs": "its own process: it SPAWNS kern rather than calling in",
    "crates/kern-common/build.rs": "a build script: cargo's own process, no sibling tests",
    "crates/kern-common/src/lib.rs": "not audited for the race yet",
    "crates/kern-compose/src/lib.rs": "not audited for the race yet",
    "crates/kern-isolation/src/lib.rs": "not audited for the race yet",
    "crates/kern-isolation/examples/sandbox_profile.rs": "an example binary, outside the crate attr",
    "crates/kern-oci/src/lib.rs": "not audited for the race yet",
}
CONFIG = ROOT / "clippy.toml"
REQUIRED_METHODS = (
    "std::env::set_var",
    "std::env::remove_var",
    "std::env::var",
    "std::env::var_os",
)


def main():
    problems = []

    text = CONFIG.read_text() if CONFIG.exists() else ""
    for m in REQUIRED_METHODS:
        if f'"{m}"' not in text:
            problems.append(f"clippy.toml no longer disallows `{m}`, so nothing stops a direct call")

    found = {}
    for f in sorted(ROOT.rglob("*.rs")):
        if "target/" in str(f) or "/fuzz/" in str(f):
            continue
        for n, line in enumerate(f.read_text().split("\n"), 1):
            if ALLOW.match(line):
                found.setdefault(str(f.relative_to(ROOT)), []).append(n)

    for path, lines in found.items():
        if path not in DECLARED:
            problems.append(
                f"{path}:{lines[0]} exempts itself from the environment lint and is not declared "
                f"in {pathlib.Path(__file__).name}. An exemption is a crate nobody is watching: "
                f"add it with its reason, or route the calls through the chokepoints"
            )
    for path in DECLARED:
        if path not in found:
            problems.append(
                f"{path} is declared as exempt but carries no allow attribute any more. Remove it "
                f"from the list: a stale exemption reads as coverage that is not there"
            )

    if problems:
        print(f"{len(problems)} problem(s):\n")
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"the environment lint is armed on {len(REQUIRED_METHODS)} methods, with "
        f"{len(DECLARED)} declared exemptions and no undeclared ones"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
