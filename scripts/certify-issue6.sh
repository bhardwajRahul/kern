#!/bin/sh
# CERTIFY THE FIX FOR ISSUE #6, on a host that has no SELinux.
#
# #6: on Fedora/RHEL with SELinux Enforcing, `pasta` is refused the open of the netns DIRECTORY it
# watches so it can quit when the namespace disappears. The refusal is `dontaudit`ed, so the audit
# log stays empty while it happens. kern retries once with `--no-netns-quit`, which is the only
# thing that drops that open, and the pod gets its NAT.
#
# WHY THIS SCRIPT EXISTS SEPARATELY FROM THE ACCEPTANCE MATRIX. The matrix already has a refusing
# pasta, but its stub refuses BOTH attempts, so it certifies only that kern REPORTS the refusal
# honestly. The case that is actually the fix - the retry SUCCEEDS and the pod ends up with working
# egress - was never exercised anywhere except by hand in a Fedora VM. A stub that refuses only the
# watching attempt reproduces the SELinux shape exactly, on any host, with no policy involved:
#
#   pasta --config-net ... /proc/N/ns/net          -> refused        (what the policy does)
#   pasta --config-net ... --no-netns-quit ...     -> succeeds       (what the retry does)
#
# and the thing being measured is then a REAL PAYLOAD off the internet, pulled from inside a box,
# through the NAT that only exists because the retry happened.
#
# Usage:
#   sh scripts/certify-issue6.sh [path-to-kern]     # default: target/release/kern
#   sh scripts/certify-issue6.sh --self-check       # the assertions, against fixed strings
#
# Exit 0 only if every case passed. A host that cannot run the POSITIVE CONTROL (a box reaching the
# internet through an unmodified pasta) SKIPS the payload cases rather than failing them, because on
# such a host a red result would say nothing about the fix.
set -u

FAIL=0
pass() { printf '    ok    %s\n' "$1"; }
fail() { printf '    FAIL  %s\n' "$1"; FAIL=$((FAIL + 1)); }
skip() { printf '    SKIP  %s\n' "$1"; }

# --- the assertions, exercisable without starting anything ---------------------------------------

# kern's own report that this pod has outbound. The four-arm sentence in `pod::network_sentence`;
# only the fully-healthy arm claims the internet.
says_outbound() { printf '%s' "$1" | grep -q 'outbound to the internet'; }

# The pod is up but has NO egress. Both the "refused" arm and the "not installed" arm say this, and
# the certification needs them to be distinguishable from success, not from each other.
says_loopback_only() { printf '%s' "$1" | grep -q 'loopback-only'; }

# The retry happened AND the first reason survived it. Reporting only the second reason would hide
# the one that names the operation a policy refused.
reports_both_reasons() {
    printf '%s' "$1" | grep -q 'netns dir open' \
        && printf '%s' "$1" | grep -q 'retried without the netns watch'
}

# HTTP/1.x 200, from a real server, fetched inside a box. A connect that succeeds proves nothing
# about a NAT: only bytes coming back do.
fetched_a_page() { printf '%s' "$1" | grep -q 'PAYLOAD_200'; }

self_check() {
    echo "  self-check: the assertions, against fixed strings"
    says_outbound "pod p: services reach each other by name + outbound to the internet (pasta)" \
        && pass "the healthy sentence is read as having outbound" \
        || fail "the healthy sentence was not read as having outbound"
    # THE NEGATIVE CONTROL THAT MATTERS: the sentence a pod gets when pasta refused and the retry
    # did not save it. If this ever reads as outbound, every green below is worthless.
    says_outbound "pod p: loopback-only - services reach each other; pasta is installed but is not running for this pod (the \`pod create\` line says why it refused)" \
        && fail "the refused-pasta sentence was read as having outbound" \
        || pass "the refused-pasta sentence is not read as having outbound"
    says_outbound "pod p: outbound is DOWN - pasta started for this pod and has since exited (create reported no problem, so look for a crash, an OOM kill, or a racing teardown)" \
        && fail "the died-pasta sentence was read as having outbound" \
        || pass "the died-pasta sentence is not read as having outbound"
    says_loopback_only "pod p: loopback-only - services reach each other; NO outbound (install \`passt\`/\`pasta\` for egress)" \
        && pass "the no-pasta sentence is read as loopback-only" \
        || fail "the no-pasta sentence was not read as loopback-only"
    says_loopback_only "pod p: services reach each other by name + outbound to the internet (pasta)" \
        && fail "the healthy sentence was read as loopback-only" \
        || pass "the healthy sentence is not read as loopback-only"
    reports_both_reasons "network: loopback-only; pasta IS installed but did not start: netns dir open: Permission denied, exiting; retried without the netns watch and it also failed: nope" \
        && pass "both reasons are recognised when the retry also failed" \
        || fail "the both-reasons line was not recognised"
    # Only the SECOND reason: the shape that would hide what a policy refused.
    reports_both_reasons "network: loopback-only; pasta IS installed but did not start: nope" \
        && fail "a single-reason line was read as reporting both" \
        || pass "a single-reason line is not read as reporting both"
    fetched_a_page "HTTP/1.1 200 OK
Content-Type: text/html
PAYLOAD_200" \
        && pass "a 200 with the marker is read as a fetched page" \
        || fail "the fetched page was not recognised"
    # A CONNECT IS NOT A FETCH. This is what a live socket to a dead NAT looks like.
    fetched_a_page "HTTP/1.1 503 Service Unavailable" \
        && fail "a 503 was read as a fetched page" \
        || pass "a 503 is not read as a fetched page"
    fetched_a_page "" \
        && fail "empty output was read as a fetched page" \
        || pass "empty output is not read as a fetched page"
    [ "$FAIL" -eq 0 ] && echo "  self-check passed" || echo "  self-check FAILED"
    exit $([ "$FAIL" -eq 0 ] && echo 0 || echo 1)
}

[ "${1:-}" = "--self-check" ] && self_check

KERN=${1:-target/release/kern}
[ -x "$KERN" ] || { echo "no kern binary at $KERN"; exit 2; }
KERN=$(CDPATH= cd "$(dirname "$KERN")" && pwd)/$(basename "$KERN")
BB=$(command -v busybox) || { echo "SKIP: busybox is needed to build a test rootfs"; exit 0; }
REAL_PASTA=$(command -v pasta) || { echo "SKIP: no pasta, so there is no retry to certify"; exit 0; }

D=$(mktemp -d) || exit 2
cleanup() {
    for p in $(cat "$D"/pods 2>/dev/null); do
        XDG_RUNTIME_DIR=$D/xdg "$KERN" pod rm "$p" >/dev/null 2>&1
    done
    rm -rf "$D"
}
trap cleanup EXIT INT TERM

XDG=$D/xdg
RF=$D/rootfs
mkdir -p "$XDG" "$RF/bin" "$RF/tmp" "$RF/proc" "$RF/dev" "$RF/etc" "$D/stub"
cp "$BB" "$RF/bin/busybox"
for l in $(ldd "$BB" 2>/dev/null | grep -oE '/[^ ]+\.so[^ ]*'); do
    [ -e "$l" ] && { mkdir -p "$RF$(dirname "$l")"; cp "$l" "$RF$l" 2>/dev/null; }
done
# Only the applets this script actually runs. `nc` and `cat` were here and unused, and an applet
# that is present but never invoked is indistinguishable from one that is missing until the day
# something calls it: this project has twice diagnosed `sh: nc: not found` as an unreachable peer.
for a in sh wget; do ln -sf busybox "$RF/bin/$a"; done

# The egress target is resolved HERE, on the host, and used by ADDRESS inside the box. Two reasons:
# the NAT and the DNS are separate halves of `setup_outbound` and a failure of either would
# otherwise read as the same red, and a test that needs the box's resolver cannot tell "no egress"
# from "no name resolution". A by-name fetch is done as well, further down, where it belongs.
TARGET_HOST=example.com
TARGET_IP=$(getent ahostsv4 "$TARGET_HOST" 2>/dev/null | awk 'NR==1{print $1}')

# Every pod this script creates is recorded so the EXIT trap can tear it down even if the script is
# interrupted between creating one and removing it. Appended to the file directly: keeping the same
# list in a shell variable as well meant two copies of one fact, and the trap can only read the file.
note_pod() { echo "$1" >> "$D/pods"; }

# Fetch through the pod, from inside a box, by IP with an explicit Host header.
#
# The box gets a FRESH NAME each call: `kern box` takes the name as its first positional, and the
# first version of this passed a PATH there and then `--rootfs` as well. Every call failed, the
# positive control read that as "this host cannot reach the internet", and the payload cases - the
# only ones that measure the fix rather than its plumbing - skipped while the script printed
# "certified". A broken probe reports the same colour as a broken subject, which is why the positive
# control exists; it caught this.
_probe_n=0
fetch_in_pod() {
    _pod=$1
    _url=$2
    _host=$3
    _probe_n=$((_probe_n + 1))
    XDG_RUNTIME_DIR=$XDG "$KERN" box "c6probe$_probe_n" --rootfs "$RF" --pod "$_pod" -- \
        /bin/busybox sh -c \
        "/bin/busybox wget -q -T 8 -O - --header 'Host: $_host' '$_url' >/dev/null 2>&1 && echo PAYLOAD_200" \
        2>/dev/null
}

# The pasta stub. Every invocation is logged, one line per call, so the NUMBER of attempts is
# counted rather than inferred from a message. `$MODE` decides what it refuses:
#   watch-only : refuse the attempt that watches the netns, exec the real pasta for the retry.
#                THE SELINUX SHAPE.
#   both       : refuse every attempt. The pod must end up with no egress.
#   other      : refuse with a DIFFERENT permission error, which must NOT be retried.
#   buried     : the refusal is the FOURTH line, under three informational ones. Measured on WSL2,
#                where pasta's first line is "Started as root, will change to nobody." A reader
#                given only the first line is told something true and useless.
#   hang       : never return. `Command::output` waits forever, so this used to hang `pod create`.
#   flood      : refuse, after ten thousand lines of noise, to check the reason is capped.
write_stub() {
    cat > "$D/stub/pasta" <<EOS
#!/bin/sh
echo "call" >> "$D/calls"
printf '%s\n' "\$*" >> "$D/argv"
MODE=$1
has_no_quit=no
for a in "\$@"; do [ "\$a" = "--no-netns-quit" ] && has_no_quit=yes; done
case "\$MODE" in
  watch-only)
      if [ "\$has_no_quit" = yes ]; then exec $REAL_PASTA "\$@"; fi
      echo "netns dir open: Permission denied, exiting" >&2; exit 1 ;;
  both)
      echo "netns dir open: Permission denied, exiting" >&2; exit 1 ;;
  other)
      echo "Couldn't open network namespace /proc/1/ns/net: Permission denied" >&2; exit 1 ;;
  buried)
      if [ "\$has_no_quit" = yes ]; then exec $REAL_PASTA "\$@"; fi
      echo "Started as root, will change to nobody." >&2
      echo "No interfaces with usable IPv6 routes" >&2
      echo "Couldn't pick external interface: disabling IPv6" >&2
      echo "netns dir open: Permission denied, exiting" >&2
      exit 1 ;;
  hang)
      sleep 600 ;;
  flood)
      i=0
      while [ \$i -lt 10000 ]; do echo "noise \$i aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" >&2; i=\$((i+1)); done
      echo "netns dir open: Permission denied, exiting" >&2; exit 1 ;;
esac
EOS
    chmod +x "$D/stub/pasta"
    : > "$D/calls"
    : > "$D/argv"
}
calls() { wc -l < "$D/calls" 2>/dev/null | tr -d ' '; }

# Live pod holders on this machine, by ARGV POSITION: argv[0] must name a kern and argv[1] must be
# the marker. Never by `pgrep -f`, which matches this script's own command line and has twice in
# this project been mistaken for a real process.
holders_now() {
    _h=0
    for _p in /proc/[0-9]*; do
        _c=$(tr '\0' '\n' < "$_p/cmdline" 2>/dev/null) || continue
        [ "$(basename "$(printf '%s' "$_c" | sed -n 1p)" 2>/dev/null)" = "kern" ] \
            && [ "$(printf '%s' "$_c" | sed -n 2p)" = "__pod-holder" ] \
            && _h=$((_h + 1))
    done
    echo "$_h"
}
HOLDERS_AT_START=$(holders_now)

echo "certify #6: the SELinux netns-dir refusal, and the retry that answers it"
echo "  kern:  $KERN"
echo "  pasta: $REAL_PASTA"
echo "  target: $TARGET_HOST ($TARGET_IP)"

# --- POSITIVE CONTROL ----------------------------------------------------------------------------
# Can a box on THIS host reach the internet through an unmodified pasta at all? Everything below
# measures the absence or presence of exactly that, so if it cannot happen here for reasons that
# have nothing to do with #6 (no route, a proxy, a firewall), the payload cases must SKIP rather
# than go red and be read as the fix failing.
echo
echo "  positive control: the harness can observe egress at all"
CAN_EGRESS=no
if [ -z "$TARGET_IP" ]; then
    skip "no IPv4 for $TARGET_HOST from this host; the payload cases cannot run"
else
    XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6ctl >/dev/null 2>&1
    note_pod c6ctl
    ctl=$(fetch_in_pod c6ctl "http://$TARGET_IP/" "$TARGET_HOST")
    if fetched_a_page "$ctl"; then
        CAN_EGRESS=yes
        pass "a box reaches the internet through an unmodified pasta"
    else
        skip "this host cannot fetch through a normal pod; the payload cases below would be meaningless"
    fi
    XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6ctl >/dev/null 2>&1
fi

# --- 1. THE FIX ITSELF ---------------------------------------------------------------------------
echo
echo "  the SELinux shape: the watch is refused, the retry is not"
write_stub watch-only
out=$(PATH="$D/stub:$PATH" XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6fix 2>&1)
note_pod c6fix

[ "$(calls)" = "2" ] \
    && pass "exactly two attempts: one refused, one retried (not a loop, not a single try)" \
    || fail "expected 2 pasta invocations, counted '$(calls)'"
grep -q -- '--no-netns-quit' "$D/argv" \
    && pass "the retry passed --no-netns-quit" \
    || fail "the retry did not pass --no-netns-quit"
# The FIRST attempt must NOT carry the flag: if it did, the refusal would never be observed on a
# host that has the policy, and the retry would be dead code that no host ever reaches.
[ "$(head -1 "$D/argv" | grep -c -- '--no-netns-quit')" = "0" ] \
    && pass "the first attempt watches the netns, so a real policy still refuses it" \
    || fail "the first attempt already carried --no-netns-quit"

says_outbound "$out" \
    && pass "kern reports outbound after the retry" \
    || fail "kern did not report outbound after the retry: $(printf '%s' "$out" | tail -1)"
[ -f "$XDG/kern/pods/c6fix/resolv.conf" ] \
    && pass "the pod's resolv.conf was written, so DNS is configured too" \
    || fail "no resolv.conf: the NAT came up but DNS did not"

# The pasta that is actually running must be the retried one. Read from the process, not from our
# own log, because the log says what was ASKED and this says what SURVIVED.
pp=$(cat "$XDG/kern/pods/c6fix/pasta.pid" 2>/dev/null || echo 0)
if [ "${pp:-0}" -gt 0 ] && tr '\0' '\n' < "/proc/$pp/cmdline" 2>/dev/null | grep -q -- '--no-netns-quit'; then
    pass "the live pasta is the retried one, running without the netns watch"
else
    fail "the running pasta is not the retried one (pid '${pp:-none}')"
fi

if [ "$CAN_EGRESS" = yes ]; then
    got=$(fetch_in_pod c6fix "http://$TARGET_IP/" "$TARGET_HOST")
    fetched_a_page "$got" \
        && pass "A REAL PAGE, fetched from inside the box through the retried NAT" \
        || fail "the pod claims outbound but no bytes came back"
    # By NAME as well: the NAT and the resolver are separate halves and the by-IP fetch above
    # cannot tell whether DNS works.
    gotn=$(fetch_in_pod c6fix "http://$TARGET_HOST/" "$TARGET_HOST")
    fetched_a_page "$gotn" \
        && pass "and by NAME, so the pod's DNS resolves through the retried NAT too" \
        || fail "the pod fetches by IP but cannot resolve a name"
else
    skip "the payload cases (positive control did not pass)"
fi

# --- 2. THE RETRIED PASTA MUST BE REAPED ---------------------------------------------------------
# THE LEAK THE FIX ITSELF INTRODUCES, and the reason this case is here rather than in the matrix. A
# pasta started with `--no-netns-quit` does not watch the namespace, so it does NOT exit when the
# pod's netns goes away: teardown's signal is the only thing that stops it. Before the retry
# existed, a missed signal was still cleaned up by pasta noticing the namespace vanish.
echo
echo "  teardown: the retried pasta does not self-exit, so it must be killed"
if [ "${pp:-0}" -gt 0 ]; then
    XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6fix >/dev/null 2>&1
    # pasta leaves on its own schedule after the signal; poll rather than assume an instant.
    gone=no
    i=0
    while [ $i -lt 40 ]; do
        kill -0 "$pp" 2>/dev/null || { gone=yes; break; }
        sleep 0.05
        i=$((i + 1))
    done
    if [ "$gone" = yes ]; then
        pass "the retried pasta is gone after pod rm"
    else
        fail "the retried pasta ($pp) survived pod rm: it never self-exits, so this leaks for the session"
        kill -9 "$pp" 2>/dev/null
    fi
else
    skip "no pasta pid was recorded, so there is nothing to reap"
fi

# --- 3. A REFUSAL THAT THE RETRY CANNOT FIX ------------------------------------------------------
echo
echo "  both attempts refused: report both reasons, claim nothing"
write_stub both
out2=$(PATH="$D/stub:$PATH" XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6both 2>&1)
note_pod c6both
[ "$(calls)" = "2" ] \
    && pass "still exactly two attempts, and it stops there" \
    || fail "expected 2 invocations when both refuse, counted '$(calls)'"
reports_both_reasons "$out2" \
    && pass "both reasons are reported, so the refused operation is still named" \
    || fail "the first reason was lost: $(printf '%s' "$out2" | tail -1)"
says_outbound "$out2" \
    && fail "a pod with no NAT claimed outbound to the internet" \
    || pass "no outbound is claimed when both attempts refused"
# POSITIVELY, not merely by the absence of the other sentence. "does not say outbound" is also
# satisfied by empty output and by a crash, so on its own it cannot tell a pod that correctly
# reports having no egress from one that reported nothing at all.
says_loopback_only "$out2" \
    && pass "and it says loopback-only, so the pod reported its state rather than going quiet" \
    || fail "the pod neither claimed outbound nor said loopback-only: $(printf '%s' "$out2" | tail -1)"
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6both >/dev/null 2>&1

# --- 4. AN UNRELATED REFUSAL MUST NOT BE RETRIED --------------------------------------------------
# The predicate is deliberately narrow. `/proc/<pid>/ns/net` and `/ns/user` are opened in BOTH runs,
# so a host that refuses THOSE refuses them identically on the retry: a second attempt could not
# succeed and would bury the real reason under a second copy of itself.
echo
echo "  a different permission error: one attempt, no retry"
write_stub other
out3=$(PATH="$D/stub:$PATH" XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6other 2>&1)
note_pod c6other
[ "$(calls)" = "1" ] \
    && pass "exactly one attempt: an unrelated refusal is not retried" \
    || fail "expected 1 invocation for an unrelated error, counted '$(calls)'"
printf '%s' "$out3" | grep -q 'network namespace' \
    && pass "the unrelated reason is reported as itself" \
    || fail "the unrelated reason was not reported: $(printf '%s' "$out3" | tail -1)"
printf '%s' "$out3" | grep -q 'retried without the netns watch' \
    && fail "an unrelated refusal was retried" \
    || pass "no retry is mentioned for an unrelated refusal"
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6other >/dev/null 2>&1

# --- 5. EDGE CASES ---------------------------------------------------------------------------------
# Everything above is the fix working or refusing cleanly. These are the shapes around it, and two
# of them were defects when this section was written.
echo
echo "  edge: the refusal is not the first line pasta prints"
# Measured on WSL2: pasta prints five lines and the FIRST is informational. If the match ran against
# one line rather than the whole stderr, a host in this shape would never retry, and the reason
# reported would be "Started as root, will change to nobody.", which is true and useless.
write_stub buried
outb=$(PATH="$D/stub:$PATH" XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6bur 2>&1)
note_pod c6bur
says_outbound "$outb" \
    && pass "a refusal on the fourth line is still found, and still retried" \
    || fail "the retry missed a refusal that was not on the first line: $(printf '%s' "$outb" | tail -1)"
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6bur >/dev/null 2>&1

echo
echo "  edge: a pasta that never returns"
# `Command::output` waits forever. A pasta that hangs therefore hung `kern pod create`, and through
# `compose up` a whole stack, with nothing printed and no way out but Ctrl-C. MEASURED against the
# SHIPPED v0.9.2 too, so the retry did not introduce it; the retry doubles the chances of meeting
# it. Bounded now, at the same ten seconds `create` already allowed the holder handshake.
write_stub hang
# THE CALL IS BOUNDED BY THE TEST TOO, and it has to be: a kern that hangs would otherwise hang
# this script rather than fail it, and a gate that never returns reports nothing at all. `timeout`
# is coreutils and present everywhere this runs; without it the case is skipped rather than made
# into a trap for whoever runs the battery next.
note_pod c6hang
if ! command -v timeout >/dev/null 2>&1; then
    skip "no timeout(1) here, and this case must not be allowed to hang the script"
else
    t0=$(date +%s)
    outh=$(timeout 45 env PATH="$D/stub:$PATH" XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6hang 2>&1)
    hang_rc=$?
    t1=$(date +%s)
    took=$((t1 - t0))
    [ "$hang_rc" -ne 124 ] && [ "$took" -lt 40 ] \
        && pass "pod create returned in ${took}s instead of hanging" \
        || fail "pod create did not return (${took}s, rc=$hang_rc): the pasta spawn is unbounded"
    says_outbound "$outh" \
        && fail "a pod whose pasta never returned claimed outbound" \
        || pass "no outbound is claimed when pasta had to be killed"
    # The whole process GROUP, not just the child: this stub is a shell script, and signalling the
    # single pid left `sleep 600` running. Real pasta is a binary and would not have shown it.
    sleep 1
    survivors=0
    for pp in /proc/[0-9]*; do
        case "$(tr '\0' ' ' < "$pp/cmdline" 2>/dev/null)" in
            *"sleep 600"*) survivors=$((survivors + 1)) ;;
        esac
    done
    [ "$survivors" -eq 0 ] \
        && pass "the wedged pasta left nothing behind, group and all" \
        || fail "$survivors process(es) survived the killed pasta"
fi
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6hang >/dev/null 2>&1

echo
echo "  edge: pasta floods stderr before refusing"
write_stub flood
outf=$(PATH="$D/stub:$PATH" XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6flood 2>&1)
note_pod c6flood
longest=$(printf '%s' "$outf" | awk '{ if (length($0) > m) m = length($0) } END { print m + 0 }')
[ "$longest" -lt 2000 ] \
    && pass "the reported reason is capped (longest line ${longest} chars)" \
    || fail "a chatty pasta flooded the output: longest line ${longest} chars"
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6flood >/dev/null 2>&1

echo
echo "  edge: the pid files name a live process that is NOT ours"
# The pid in `pasta.pid` or `holder` can be recycled by anything on the host. teardown signals both,
# and one of those signals is a SIGKILL, so being wrong here is not recoverable. A `sleep` stands in
# for the innocent bystander.
sleep 300 &
victim=$!
XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6pid --no-outbound >/dev/null 2>&1
echo "$victim" > "$XDG/kern/pods/c6pid/pasta.pid"
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6pid >/dev/null 2>&1
sleep 0.3
kill -0 "$victim" 2>/dev/null \
    && pass "a stranger named by pasta.pid is not killed" \
    || fail "teardown killed an unrelated process through pasta.pid"

XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6pid2 --no-outbound >/dev/null 2>&1
# Remove the netns file so the inode fast path cannot answer and the argv fallback decides.
rm -f "$XDG/kern/pods/c6pid2/netns"
# THE TEST REAPS WHAT THE TEST BREAKS. Overwriting the holder file destroys the only record that
# names this pod's real holder, so kern cannot reach it afterwards and it would run until logout.
# That is a limitation worth stating rather than a defect to report: nothing links a pod dir to its
# holder once the pid is gone from it. What matters here is that the battery not become the thing
# that litters the machine, because `pod ls` and the acceptance matrix both count live kern
# processes, and a leaking test turns their counts into noise.
real_holder2=$(cat "$XDG/kern/pods/c6pid2/holder" 2>/dev/null)
echo "$victim" > "$XDG/kern/pods/c6pid2/holder"
XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6pid2 >/dev/null 2>&1
sleep 0.3
kill -0 "$victim" 2>/dev/null \
    && pass "a stranger named by the holder file is not killed either" \
    || fail "teardown killed an unrelated process through the holder file"
kill "$victim" 2>/dev/null
[ -n "${real_holder2:-}" ] && kill -9 "$real_holder2" 2>/dev/null

echo
echo "  edge: the pid files hold values that are not pids"
# `kill(0, ...)` signals the caller's own process group and `kill(-1, ...)` signals every process it
# may signal. A `0` or a `-1` in one of these files must never reach `kill`, so this asserts that
# teardown still succeeds AND that this script's own shell is alive afterwards, which is the thing
# `kill(0, ...)` would have taken out.
degenerate_ok=yes
for bad in 0 -1 99999999999999999999 "" "not-a-pid" "12 34"; do
    XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6bad --no-outbound >/dev/null 2>&1
    # Read the real pid BEFORE clobbering the file, for the same reason as the case above: the
    # write is what makes the holder unreachable, so this script owns reaping it.
    real_bad=$(cat "$XDG/kern/pods/c6bad/holder" 2>/dev/null)
    printf '%s' "$bad" > "$XDG/kern/pods/c6bad/pasta.pid"
    printf '%s' "$bad" > "$XDG/kern/pods/c6bad/holder"
    XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6bad >/dev/null 2>&1 || degenerate_ok=no
    kill -0 $$ 2>/dev/null || degenerate_ok=no
    [ -d "$XDG/kern/pods/c6bad" ] && degenerate_ok=no
    case "${real_bad:-}" in ''|*[!0-9]*) ;; *) kill -9 "$real_bad" 2>/dev/null ;; esac
done
[ "$degenerate_ok" = yes ] \
    && pass "six degenerate pid-file values: torn down, nothing signalled, this shell alive" \
    || fail "a degenerate pid-file value broke teardown or signalled something"

# --- 6. THE REAL THING, where a policy and not a stub does the refusing -----------------------------
# Everything above reproduces #6's SHAPE with a stub, which is what makes it runnable anywhere. This
# block runs only where the actual policy is in force, uses the REAL pasta, and is the only case in
# the file where nothing is simulated.
#
# The discriminator is the flag on the SURVIVING pasta. A host can be Enforcing and still not
# exhibit #6 (no `passt-selinux` installed, a newer policy, a different label), and on such a host
# the first attempt succeeds and the retry never runs. Passing there would be a green that means
# "this host is fine", not "the fix works", so it SKIPS and says which it was.
echo
echo "  real SELinux: the policy refuses, not a stub"
enforce=$(cat /sys/fs/selinux/enforce 2>/dev/null || echo "")
if [ "$enforce" != "1" ]; then
    skip "no SELinux in Enforcing here (read '${enforce:-no selinuxfs}'); cases 1-4 covered the shape"
else
    XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6real >/dev/null 2>&1
    outr=$(XDG_RUNTIME_DIR=$XDG "$KERN" pod create c6real 2>&1)
    note_pod c6real
    rp=$(cat "$XDG/kern/pods/c6real/pasta.pid" 2>/dev/null || echo 0)
    rflag=no
    [ "${rp:-0}" -gt 0 ] && tr '\0' '\n' < "/proc/$rp/cmdline" 2>/dev/null \
        | grep -q -- '--no-netns-quit' && rflag=yes
    if [ "$rflag" = no ] && says_outbound "$outr"; then
        skip "this Enforcing host does not refuse the netns-dir open, so #6 does not reproduce here"
    else
        says_outbound "$outr" \
            && pass "UNDER ENFORCING, with the real policy: the pod has outbound" \
            || fail "under Enforcing the pod has no outbound: $(printf '%s' "$outr" | tail -1)"
        [ "$rflag" = yes ] \
            && pass "and the live pasta carries --no-netns-quit, so the POLICY refused the first attempt" \
            || fail "the pod came up without the retry, so this is not the #6 path"
        if [ -n "$TARGET_IP" ]; then
            gotr=$(fetch_in_pod c6real "http://$TARGET_IP/" "$TARGET_HOST")
            fetched_a_page "$gotr" \
                && pass "A REAL PAGE, under Enforcing, through the NAT the retry rescued" \
                || fail "under Enforcing the pod claims outbound but no bytes came back"
        fi
    fi
    XDG_RUNTIME_DIR=$XDG "$KERN" pod rm c6real >/dev/null 2>&1
fi

# --- 7. THE BATTERY ITSELF LEAVES NOTHING BEHIND ---------------------------------------------------
# Counted, because it was not. An earlier version of the cases above leaked SEVEN pod holders per
# run by clobbering the file that names them, and seven orphaned holders is exactly what turned up
# as an unexplained failure in the acceptance matrix, which counts live kern processes. A test suite
# that litters makes every other suite's counts unreadable, so this asserts on its own footprint.
echo
echo "  the battery's own footprint"
sleep 1
holders_end=$(holders_now)
[ "$holders_end" -le "$HOLDERS_AT_START" ] \
    && pass "no pod holder leaked (${HOLDERS_AT_START} before, ${holders_end} after)" \
    || fail "leaked $((holders_end - HOLDERS_AT_START)) pod holder(s): ${HOLDERS_AT_START} before, ${holders_end} after"

echo
if [ "$FAIL" -eq 0 ]; then
    echo "  #6 certified: every case passed"
else
    echo "  #6 NOT certified: $FAIL case(s) failed"
fi
exit $([ "$FAIL" -eq 0 ] && echo 0 || echo 1)
