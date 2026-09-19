#!/bin/sh
# EVERYTHING CI WILL SAY, SAID HERE FIRST, IN ABOUT TEN SECONDS.
#
# WHY THIS EXISTS, measured. The local checks were never the expensive part: the whole fast set
# below costs ~9 s, while ONE round of CI costs ~6.5 minutes. One session spent five red rounds -
# about 33 minutes of waiting - finding one defect per push, and not one of them needed a runner to
# be found. The cost is not running checks, it is learning about a defect from CI.
#
#   sh scripts/pre-push.sh          fast set, ~9 s: run it as often as you like
#   sh scripts/pre-push.sh --full   + the integration suite, ~38 s: run it before a push
#
# BARE EXIT CODES, never a pipe: a gate whose status is swallowed by `| tee` reports success it did
# not earn.

set -u
cd "$(dirname "$0")/.." || exit 2
FULL=0
[ "${1:-}" = "--full" ] && FULL=1
fail=0

step() {
    name=$1; shift
    t0=$(date +%s%N)
    if "$@" >/tmp/prepush.log 2>&1; then
        printf '  \033[32mok\033[0m   %-38s %5.1fs\n' "$name" "$(( ($(date +%s%N)-t0)/1000000 ))e-3"
    else
        printf '  \033[31mFAIL\033[0m %-38s %5.1fs\n' "$name" "$(( ($(date +%s%N)-t0)/1000000 ))e-3"
        sed 's/^/       /' /tmp/prepush.log | tail -25
        fail=1
    fi
}

# THE ORDER IS CHEAPEST-FIRST so the fastest thing that can be wrong says so first.
step "fmt"                     cargo fmt --all --check
step "unit tests"              cargo test -q -p getkern --bin kern
# The runner executes as an ordinary user inside a cgroup it cannot write. A developer shell sits in
# its own delegated scope, so every assertion that assumes "a process may enter its own cgroup"
# passes here and fails there. This runs the unit tests in THAT shape.
step "unit tests, CI host shape" sh scripts/as-ci-host.sh cargo test -q -p getkern --bin kern
step "clippy -D warnings"      cargo clippy --release --workspace --all-targets
for g in no-ai-slop stale-numbers docker-vocabulary md-links flat-continuation \
         test-env-lock progress-is-tty-gated injection-declared registry-classified \
         tracked-paths-sane; do
    step "$g" python3 "scripts/$g.py"
done
# NO SEPARATE EM-DASH STEP, and the reason is the whole point of this file. The first version had
# one, spelled `grep --include="*.md"`, and it was WEAKER than the gate it paraphrased: `no-ai-slop`
# scans every tracked file for that character, not just markdown. So this script passed itself green
# while carrying an em-dash in this very line, and CI - which runs the real gate - went red. A fast
# script that RESTATES a check will drift below it and hand out false greens; it must CALL it. Every
# step above is the same script CI runs, by name.
#
# IT READS WHAT GIT TRACKS, so `git add` your new files before trusting a green from it. An
# untracked file is invisible to these gates and to CI alike, right up to the commit that adds
# it. Verified both ways: an em-dash in a staged `.sh` and in a tracked `.rs` each turn this red.

if [ "$FULL" -eq 1 ]; then
    step "integration suite" cargo test -q -p getkern
fi

echo
if [ "$fail" -eq 0 ]; then
    [ "$FULL" -eq 1 ] && echo "green: this is what CI will say." \
                      || echo "green on the fast set. Before pushing: sh scripts/pre-push.sh --full"
    exit 0
fi
echo "RED. Fix it here; a CI round costs ~6.5 minutes and says the same thing."
exit 1
