#!/bin/sh
# RUN A COMMAND IN A CGROUP THIS PROCESS CANNOT WRITE, which is the shape GitHub's runner has and
# a developer machine does not.
#
# WHY THIS EXISTS. Three CI rounds on one branch were spent finding, one per round, tests that had
# baked in a property of the machine they were written on. The runner executes the job as an ordinary
# user inside `/sys/fs/cgroup/system.slice/hosted-compute-agent.service`, a cgroup owned by root: the
# `cgroup.procs` there exists and is NOT writable. A developer shell sits in its own delegated scope
# and can write its own, so every assertion that quietly assumed "a process may enter its own cgroup"
# passed locally and failed on push.
#
# That property is the one kern's delegation gate exists for, so the tests most likely to encode its
# opposite are exactly the tests about it.
#
# HOW. Create a cgroup, ENTER IT FIRST, then drop write permission on its `cgroup.procs`. The order
# matters: chmod before entering locks you out of your own directory, which is a different failure
# and reads like the harness being broken.
#
# Not a substitute for CI: it reproduces ONE host property, the one that has actually bitten. A green
# run here means that class is covered, not that the runner will agree about everything.
#
#   sh scripts/as-ci-host.sh cargo test --all
#   sh scripts/as-ci-host.sh --self-check

set -u

if [ "${1:-}" = "--self-check" ]; then
    # The harness has to be able to FAIL, or a green run through it means nothing. The check is that
    # the shape it builds is actually the restricted one, measured from inside.
    echo "  self-check: the shape is built and is really restricted"
    out=$(sh "$0" sh -c 'm=$(sed -n "s/^0:://p" /proc/self/cgroup); [ -w "/sys/fs/cgroup$m/cgroup.procs" ] && echo WRITABLE || echo RESTRICTED' 2>&1)
    case "$out" in
        *RESTRICTED*) echo "    ok    a command inside sees its own cgroup.procs as NOT writable"; exit 0 ;;
        *WRITABLE*)   echo "    FAIL  the shape was not built: the command could still write its own cgroup.procs"; exit 1 ;;
        *)            echo "    SKIP  could not build the shape here: $out"; exit 0 ;;
    esac
fi

[ $# -ge 1 ] || { echo "usage: $0 <command...>   |   $0 --self-check"; exit 2; }

uid=$(id -u)
base="/sys/fs/cgroup/user.slice/user-$uid.slice/user@$uid.service"
[ -d "$base" ] || {
    # A SKIP WITH ITS REASON: without a delegated user manager there is no directory this user may
    # create a cgroup in, so the shape cannot be built. Saying so beats reporting a pass.
    echo "  SKIP  no delegated user@$uid.service here, so the restricted shape cannot be built"
    exec "$@"
}

# WHERE TO GO BACK TO, captured before moving. Not `$base`: cgroup v2 forbids processes in a cgroup
# that has children ("no internal processes"), and `user@<uid>.service` always has them, so every
# write of a pid there fails with EBUSY. Silenced, that failure meant nothing ever left the shape and
# the directory could never be removed - three survivors reported after a command that forked nothing
# of its own. The cgroup this script started in held this shell, so by construction it can hold it
# again.
home_cg="/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)"

dir="$base/kern-ci-shape-$$"
mkdir -p "$dir" 2>/dev/null || { echo "  SKIP  cannot create a cgroup under $base"; exec "$@"; }

# Restore write permission and leave, then remove the directory. A cgroup with a process still in it
# cannot be removed, and one left behind with 0500 on its `cgroup.procs` is a trap for the next run:
# the next invocation cannot enter its own directory and reports a harness failure for someone else's
# leftovers.
#
# RETRIED AND THEN REPORTED, because a single `rmdir` is not enough and silence is worse than either.
# Anything the command under test started inherits this cgroup, so a suite that leaks a box leaves a
# process here after the command returns: measured, seven directories after one afternoon, two of
# them still holding live `kern` and `pasta` processes. The retry covers the ordinary case of a child
# that is on its way out; what survives it is named, because a leak this harness merely hid would be
# a leak nobody could see.
cleanup() {
    chmod 0700 "$dir/cgroup.procs" 2>/dev/null
    # THIS SHELL LEAVES FIRST, and the order is the whole fix. Draining the file spawns subshells for
    # the read and for `wc`, and a child is born in its parent's cgroup: draining while still inside
    # put new processes in faster than the loop took them out, so a directory that held nothing but
    # the harness reported three survivors and was never removed. Out first, and every child after
    # that is born in `base`.
    echo $$ > "$home_cg/cgroup.procs" 2>/dev/null
    # Then anything the command under test left behind, which is the only case that can remain.
    while read -r p; do
        [ -n "$p" ] && echo "$p" > "$home_cg/cgroup.procs" 2>/dev/null
    done < "$dir/cgroup.procs" 2>/dev/null
    i=0
    while [ $i -lt 20 ]; do
        rmdir "$dir" 2>/dev/null && return
        sleep 0.25
        i=$((i + 1))
    done
    left=$(wc -l < "$dir/cgroup.procs" 2>/dev/null || echo "?")
    echo "  NOTE: $dir survives with $left process(es) still in it, left by the command that ran here"
}
trap cleanup EXIT INT TERM

echo $$ > "$dir/cgroup.procs" 2>/dev/null || { echo "  SKIP  could not enter $dir"; exit 0; }
chmod 0500 "$dir/cgroup.procs" 2>/dev/null || { echo "  SKIP  could not drop write on cgroup.procs"; exit 0; }

mine=$(sed -n 's/^0:://p' /proc/self/cgroup)
if [ -w "/sys/fs/cgroup$mine/cgroup.procs" ]; then
    echo "  SKIP  the shape did not take: cgroup.procs is still writable"
    exit 0
fi

"$@"
