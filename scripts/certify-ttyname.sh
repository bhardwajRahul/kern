#!/bin/sh
# CERTIFY THAT A BOX'S CONTROLLING TERMINAL HAS A NAME, on this distro's kernel.
#
# THE DEFECT. kern used to allocate the `-it` PTY pair on the HOST and hand the slave to the box as
# its stdio. The box mounts its own private devpts at `/dev/pts`, which does not contain that node,
# so inside the box `isatty(0)` succeeded while `readlink /proc/self/fd/0` gave the HOST's path and
# `stat` of it failed. musl's `ttyname_r` is exactly that readlink plus a stat, with no fallback, so
# it returned ENOENT and `tty(1)` printed "not a tty" on every alpine box; glibc's scans `/dev` and
# found the `/dev/console` bind, so the same box looked correct on a Debian image. podman prints
# `/dev/pts/0`.
#
# THE FIX being certified: the pair is opened from the BOX's own `/dev/ptmx` and the master is passed
# back to the CLI over a socketpair with SCM_RIGHTS.
#
# WHY THIS RUNS PER DISTRO rather than once. The fix depends on the kernel letting an unprivileged
# user namespace mount `devpts` with `ptmxmode`, on `TIOCSPTLCK`/`TIOCGPTN` behaving, and on no LSM
# refusing the slave open. Those are kernel and policy properties, and SELinux Enforcing is exactly
# the shape that broke pasta in issue #6 without a word in the audit log.
#
# The probe is a STATIC MUSL binary: musl is the library the defect was visible under, and static
# means the box needs no loader, so a bare busybox rootfs is enough and no distro package is
# involved in the answer.
#
# THE PROBE IS BUILT HERE when one is not supplied, so this runs from a fresh clone. It is a few
# lines of C compiled STATICALLY: the box rootfs below holds nothing but the probe, so a dynamic
# binary would need a loader and libraries that are deliberately not there.
#
# musl is preferred and glibc is accepted, and the difference is worth stating. The OLD defect was
# only VISIBLE under musl, because glibc's `ttyname_r` falls back to scanning `/dev` and found the
# `/dev/console` bind. But this certificate treats `/dev/console` as a FAILURE, so a glibc probe
# still catches a regression to the old behaviour - it just reports it by the name glibc gives it.
#
#   sh certify-ttyname.sh <path-to-kern> [path-to-probe]
#   sh certify-ttyname.sh --self-check

set -u
fail=0
ok()   { printf '    ok    %s\n' "$1"; }
bad()  { printf '    FAIL  %s\n' "$1"; fail=1; }
skip() { printf '    SKIP  %s\n' "$1"; }

# `ttyname_r(0)   = /dev/pts/3` is a pass; a FAILED line or a `/dev/console` is not. `/dev/console`
# is called out separately because it is the OLD glibc-only behaviour: reading it as a pass is how
# this certificate would silently accept the very bug it exists to catch.
resolved() { echo "$1" | grep -q 'ttyname_r(0)   = /dev/pts/'; }
console()  { echo "$1" | grep -q 'ttyname_r(0)   = /dev/console'; }
isatty()   { echo "$1" | grep -q 'isatty(0)      = 1'; }

if [ "${1:-}" = "--self-check" ]; then
    echo "  self-check: the assertions, against fixed strings"
    resolved 'ttyname_r(0)   = /dev/pts/3' && ok "a devpts name is a pass" || bad "devpts name"
    resolved 'ttyname_r(0)   FAILED rc=2'  && bad "a failure must not pass" || ok "a failure is not a pass"
    resolved 'ttyname_r(0)   = /dev/console' && bad "console must not pass" || ok "the old glibc-only /dev/console is not a pass"
    console  'ttyname_r(0)   = /dev/console' && ok "console is recognised, so it can be named in the report" || bad "console detect"
    isatty   'isatty(0)      = 1' && ok "isatty is read" || bad "isatty"
    isatty   'isatty(0)      = 0' && bad "a non-tty must not read as a tty" || ok "a non-tty is not read as a tty"
    [ $fail -eq 0 ] && echo "  self-check passed" || echo "  self-check FAILED"
    exit $fail
fi

KERN="${1:-}"
PROBE="${2:-}"
[ -x "$KERN" ] || { echo "usage: $0 <kern> [probe]   |   $0 --self-check"; exit 2; }

BUILT=""
if [ -z "$PROBE" ] || [ ! -f "$PROBE" ]; then
    BUILT=$(mktemp -d /var/tmp/ttyname-build.XXXXXX) || exit 2
    cat > "$BUILT/probe.c" <<'CEOF'
#include <stdio.h>
#include <unistd.h>
int main(void) {
    char b[256];
    printf("isatty(0)      = %d\n", isatty(0));
    int rc = ttyname_r(0, b, sizeof b);
    if (rc == 0) { printf("ttyname_r(0)   = %s\n", b); }
    else         { printf("ttyname_r(0)   FAILED rc=%d\n", rc); }
    return 0;
}
CEOF
    for cc in musl-gcc cc gcc clang; do
        command -v "$cc" >/dev/null 2>&1 || continue
        if "$cc" -static -O1 -o "$BUILT/probe" "$BUILT/probe.c" 2>/dev/null; then
            PROBE="$BUILT/probe"
            break
        fi
    done
    if [ -z "$PROBE" ] || [ ! -f "$PROBE" ]; then
        # A SKIP WITH ITS REASON, never a pass: this host cannot answer the question, and saying so
        # is the result. A statically linked C compiler is the only thing missing.
        echo "  SKIP  no compiler able to produce a static binary (tried musl-gcc, cc, gcc, clang)"
        rm -rf "$BUILT"
        exit 0
    fi
fi
# A BARE NAME IS NOT A COMMAND, and `[ -x ]` accepts it anyway. `certify-ttyname.sh kern-new ...`
# passed the check above and then died with "not found" on every case, which the battery reported as
# "the box got no terminal" - a real-looking failure with the wrong cause. Resolve it here so the
# report can never blame the box for the caller's argument.
case "$KERN" in */*) ;; *) KERN="./$KERN" ;; esac
[ -x "$KERN" ] || { echo "$1: not executable as a command"; exit 2; }

# A rootfs with nothing in it but the probe. `--rootfs` rather than `--image` on purpose: no pull, no
# registry, no network, so a distro that cannot reach the internet still answers the question.
R=$(mktemp -d /var/tmp/ttyname-rootfs.XXXXXX) || exit 2
mkdir -p "$R/bin" "$R/proc" "$R/dev" "$R/sys" "$R/tmp" || exit 2
cp "$PROBE" "$R/bin/probe" || exit 2
chmod +x "$R/bin/probe"
# A shell for case 2 to keep a box ALIVE to exec into. The probe prints and exits, so a detached box
# running it is gone before the exec, and the case would report SKIP for a reason that is an artifact
# of the battery rather than a property of the host. Static busybox: no loader, no distro package.
BB=""
for c in ./busybox-static /usr/local/bin/busybox /bin/busybox "$(dirname "$PROBE")/busybox-static"; do
    [ -x "$c" ] && { cp "$c" "$R/bin/busybox" && chmod +x "$R/bin/busybox" && BB=/bin/busybox; break; }
done

echo "  certify: the box's controlling terminal must have a name"
printf '    host:    '; (. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-?}")
printf '    kernel:  '; uname -r
printf '    selinux: '; if [ -r /sys/fs/selinux/enforce ]; then cat /sys/fs/selinux/enforce; else echo absent; fi

# CASE 1: kern box -it. The box's PID 1 is the probe itself.
OUT=$("$KERN" box ttyc1 -it --rootfs "$R" -- /bin/probe 2>&1)
if isatty "$OUT"; then
    if resolved "$OUT"; then
        ok "box -it: $(echo "$OUT" | grep 'ttyname_r' | sed 's/^ *//')"
    elif console "$OUT"; then
        bad "box -it: resolved to /dev/console, which is the pre-fix glibc-only path"
    else
        bad "box -it: $(echo "$OUT" | grep 'ttyname_r' | sed 's/^ *//' || echo 'no ttyname line at all')"
    fi
else
    # NOT a pass and NOT a silent skip: `-it` that produces no terminal at all is its own defect,
    # and on a host where boxes cannot start the earlier batteries will have said so already.
    bad "box -it: the box got no terminal at all (isatty=0), so the name could not be tested"
fi

# CASE 2: kern exec -it into a live box. A DIFFERENT code path: the pair is opened after `setns`
# rather than during the mount sequence, so a kernel that allows one can refuse the other.
if [ -n "$BB" ]; then
    "$KERN" box ttyc2 --rootfs "$R" -d -- "$BB" sleep 60 >/dev/null 2>&1
    sleep 1
fi
if [ -n "$BB" ] && "$KERN" ps 2>/dev/null | grep -q ttyc2; then
    # KERN_ALLOW_UNCAPPED=1 for this case only, and it is not a weakening of the test. On a host
    # where the caller sits outside the tree kern's cgroups live in, `kern exec` fails CLOSED by
    # design and never reaches the terminal code: measured on Fedora 44 cloud, where the refusal is
    # "could not be placed in the box's cgroup". That posture is what `certify-issue6` and the
    # acceptance matrix are for. Re-measuring it here would report the same host property a third
    # time and hide the answer this battery exists to give.
    OUT2=$(KERN_ALLOW_UNCAPPED=1 "$KERN" exec -it ttyc2 -- /bin/probe 2>&1)
    if isatty "$OUT2"; then
        if resolved "$OUT2"; then
            ok "exec -it: $(echo "$OUT2" | grep 'ttyname_r' | sed 's/^ *//')"
        elif console "$OUT2"; then
            bad "exec -it: resolved to /dev/console, which is the pre-fix glibc-only path"
        else
            bad "exec -it: $(echo "$OUT2" | grep 'ttyname_r' | sed 's/^ *//' || echo 'no ttyname line at all')"
        fi
    else
        # NAME WHICH OF THE TWO IT IS. No `isatty=` line can mean the exec never ran (a refusal, a
        # missing binary, an LSM) or that it ran without a terminal, and those need opposite fixes.
        # The first version of this case reported both as "no terminal", which on Fedora sent me
        # looking at the PTY code for what turned out to be something else entirely.
        if echo "$OUT2" | grep -q 'isatty(0)'; then
            bad "exec -it: ran, but with NO terminal (isatty=0)"
        else
            bad "exec -it: the command did not run: $(echo "$OUT2" | head -2 | tr '\n' ' ' | cut -c1-120)"
        fi
    fi
else
    # The box is a `-d` probe that prints and exits, so it may already be gone. Say which, rather
    # than report a pass for a case that never ran.
    if [ -z "$BB" ]; then
        skip "exec -it: no static busybox to hold a box open (looked for busybox-static beside the probe)"
    else
        bad "exec -it: the box that should have stayed alive is not running"
    fi
fi
"$KERN" stop ttyc2 >/dev/null 2>&1
"$KERN" gc >/dev/null 2>&1
rm -rf "$R"
[ -n "$BUILT" ] && rm -rf "$BUILT"

# THE FOOTPRINT, on the same reasoning as certify-issue6: a battery that leaks boxes turns the next
# run's numbers into a mystery, and this one starts two.
LEFT=$("$KERN" ps 2>/dev/null | grep -c 'ttyc' || true)
[ "$LEFT" -eq 0 ] && ok "no box left behind" || bad "$LEFT box(es) left behind"

if [ $fail -eq 0 ]; then
    echo "  ttyname certified"
else
    echo "  ttyname NOT certified"
fi
exit $fail
