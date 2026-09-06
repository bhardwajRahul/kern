/**
 * The half that needs no pi, exercised without it.
 *
 * Runs against `./pure.ts`, which imports nothing from `@earendil-works/pi-coding-agent`, so this
 * file is the one an auditor can run holding only the npm tarball. CI deletes the peer before
 * calling it: a split that is only declared is not a split.
 *
 *   node --experimental-strip-types test-pure.ts
 */
import { refuseOutsideWorkspace, globMatches, guestHome, GUEST_WORKSPACE } from "./pure.ts";

let bad = 0;
let n = 0;
function ok(cond: boolean, what: string, saw?: unknown): void {
	n++;
	console.log(`  ${cond ? "PASS" : "FAIL"}  ${what}${cond ? "" : `  <- ${JSON.stringify(saw)}`}`);
	if (!cond) bad++;
}
function refuses(input: unknown, what: string): void {
	let threw = false;
	let saw: unknown;
	try {
		saw = refuseOutsideWorkspace(input as string);
	} catch {
		threw = true;
	}
	ok(threw, what, saw);
}
function yields(input: string, want: string): void {
	let saw: unknown;
	try {
		saw = refuseOutsideWorkspace(input);
	} catch (e) {
		saw = `threw: ${e}`;
	}
	ok(saw === want, `${JSON.stringify(input)} -> ${JSON.stringify(want)}`, saw);
}

console.log("\n== the gate accepts what is inside");
yields(GUEST_WORKSPACE, "");
yields(`${GUEST_WORKSPACE}/a/b`, "a/b");
yields(`${GUEST_WORKSPACE}/a/../b`, "b");
yields(`  ${GUEST_WORKSPACE}/a  `, "a");

console.log("\n== and refuses everything else");
// The prefix attack: a sibling directory whose name STARTS with the workspace's.
refuses("/workspaceevil", "a sibling path that merely starts with the workspace name");
refuses("/workspaceevil/x", "a file under that sibling");
refuses(`${GUEST_WORKSPACE}/../etc/passwd`, "one .. above the workspace");
refuses(`${GUEST_WORKSPACE}/../../../../etc/passwd`, "four of them");
refuses(`${GUEST_WORKSPACE}/..`, "the parent itself");
refuses("/etc/passwd", "an unrelated absolute path");
refuses("workspace/x", "a relative path");
// Case matters on Linux, and a gate that lowercased would let this through.
refuses("/Workspace/a", "the same name in another case");
// Not strings. These arrive from a model, so they are reachable inputs, and until 0.1.2 they threw
// a TypeError from .trim() rather than being refused.
refuses(null, "null");
refuses(undefined, "undefined");
refuses(42, "a number");
refuses({}, "an object");
// A NUL truncates the path in every syscall taking a C string, so the name the gate reads and the
// name the kernel opens are different strings.
refuses(`${GUEST_WORKSPACE}/a\0b`, "a NUL byte");
// A newline is NOT refused: it is legal in a POSIX filename and the shell path single-quotes.
yields(`${GUEST_WORKSPACE}/a\nb`, "a\nb");

console.log("\n== the glob matcher");
// The order is (glob, candidate). Written the other way round first, and two of the four assertions
// passed anyway, which is what a reversed-argument test looks like from the outside.
ok(globMatches("*.txt", "a.txt"), "*.txt matches a.txt");
ok(!globMatches("*.txt", "sub/a.txt"), "*.txt does not cross a directory");
ok(globMatches("**/*.txt", "sub/a.txt"), "**/*.txt does");
ok(!globMatches("*.txt", "a.md"), "*.txt rejects another extension");

console.log("\n== guestHome");
ok(guestHome(GUEST_WORKSPACE) === GUEST_WORKSPACE, "the workspace is its own home by default");

console.log(`\n  ${n - bad}/${n} PASS, ${bad} FAIL`);
process.exit(bad === 0 ? 0 : 1);
