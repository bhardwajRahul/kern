/**
 * The seven tools AS REGISTERED, driven through their own execute().
 *
 * The suite whose absence let two grep defects ship three days apart. The other files here test the
 * pieces: the gate as a function, the box through the SDK, the registration shape, and pi's dispatch.
 * None of them called `execute` with a hostile argument, so grep was first built against a path that
 * does not exist on the host (broken on every call) and then against one that does (unconfined: an
 * external audit read /etc/passwd through an absolute `path`, and host content through a workspace
 * symlink). Both passed every test in this directory.
 *
 *   node --experimental-strip-types test-tools.ts
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import activate from "./index.ts";

const ws = fs.mkdtempSync(path.join(os.tmpdir(), "kern-pi-tools-"));
const outside = fs.mkdtempSync(path.join(os.tmpdir(), "kern-pi-outside-"));
fs.writeFileSync(path.join(outside, "secret.txt"), "MARKER_OUTSIDE\n");
fs.writeFileSync(path.join(ws, "proj.txt"), "MARKER_PROJ\nsecond line\n");
fs.mkdirSync(path.join(ws, "sub"));
fs.writeFileSync(path.join(ws, "sub", "deep.txt"), "MARKER_DEEP\n");
fs.symlinkSync(path.join(outside, "secret.txt"), path.join(ws, "link-out.txt"));
fs.symlinkSync("/etc", path.join(ws, "link-etc"));
process.chdir(ws);

// ACTIVATE AFTER THE CHDIR. The extension captures `process.cwd()` as the workspace when it is
// activated, so activating first and moving later gave it the wrong root, and the three positive
// controls failed while every refusal still passed. A test whose negative half passes for the wrong
// reason is the shape to watch for.
const tools: Record<string, any> = {};
activate({ registerTool: (t: any) => (tools[t.name] = t), registerCommand: () => {}, on: () => {} } as never);

let bad = 0;
let n = 0;
function ok(cond: boolean, what: string, saw?: unknown): void {
	n++;
	console.log(`  ${cond ? "PASS" : "FAIL"}  ${what}${cond ? "" : `  <- ${JSON.stringify(saw)?.slice(0, 200)}`}`);
	if (!cond) bad++;
}
async function call(tool: string, params: Record<string, unknown>): Promise<string> {
	try {
		const r = await tools[tool].execute("t", params, new AbortController().signal, () => {}, undefined);
		return JSON.stringify(r);
	} catch (e) {
		return `THREW ${String(e).split("\n")[0]}`;
	}
}
const refused = (s: string) => s.startsWith("THREW") || /refus|escape|outside|denied/i.test(s);

console.log("\n== grep finds what is inside the project");
ok((await call("grep", { pattern: "MARKER_PROJ" })).includes("proj.txt"), "a match in the workspace root");
ok((await call("grep", { pattern: "MARKER_DEEP" })).includes("sub/deep.txt"), "a match in a subdirectory");
ok((await call("grep", { pattern: "MARKER_PROJ", path: "." })).includes("proj.txt"), "an explicit '.' path");
ok((await call("grep", { pattern: "nothing-matches-this" })).includes("No matches"), "and says so when nothing matches");
// kern writes `.kern-env.<boxid>` into the box's workspace. It is per call, so it would be noise in
// every result and a different name each time.
const everything = await call("grep", { pattern: "." });
ok(!everything.includes(".kern-env"), "kern's own scaffolding is not in the results", everything.slice(0, 160));

console.log("\n== and cannot reach anything outside it");
// Every one of these returned host content on 0.1.3. They are the audit's five, verbatim.
ok(refused(await call("grep", { pattern: "^root:", path: "/etc/passwd" })), "an absolute path to /etc/passwd");
ok(refused(await call("grep", { pattern: "MARKER_OUTSIDE", path: outside })), "an absolute path to another directory");
// A symlink is not refused: the search runs in the box, so the link resolves inside the box's own
// filesystem. What must be true is that no HOST content comes back, and "refused" is the wrong
// assertion for it. Both of these returned host bytes on 0.1.3.
const viaLink = await call("grep", { pattern: "MARKER_OUTSIDE", path: "link-out.txt" });
ok(!viaLink.includes("MARKER_OUTSIDE"), "a symlink to a host file yields nothing of it", viaLink);
// The host's own username is in the host's /etc/passwd and not in the box image's, which is what
// tells the two files apart: their root lines are byte-identical and discriminate nothing.
const viaEtc = await call("grep", { pattern: os.userInfo().username, path: "link-etc/passwd" });
ok(!viaEtc.includes(os.userInfo().username), "a symlink to /etc reaches the BOX's /etc, not this host's", viaEtc);
ok(refused(await call("grep", { pattern: "MARKER_OUTSIDE", path: "../" })), "a .. above the workspace");

console.log("\n== the shape of what grep returns");
fs.writeFileSync(path.join(ws, "ctx.txt"), "L1\nL2 MARKER_CTX\nL3\n");
fs.mkdirSync(path.join(ws, "dashdir"));
fs.writeFileSync(path.join(ws, "dashdir", "dash-name.txt"), "also MARKER_CTX\n");
// grep omits the filename when it is handed a single file, so `path: "ctx.txt"` came back as
// `2:L2` and an agent could not tell which file a line was from. `-H` forces it.
ok((await call("grep", { pattern: "MARKER_CTX", path: "ctx.txt" })).includes("ctx.txt:2:"),
   "a single-file search still names the file");
// Context lines use `-` where match lines use `:`, and grep emits a bare `--` between groups.
// Reading the path as "everything before the first colon" mangled every context line.
const withCtx = await call("grep", { pattern: "MARKER_CTX", path: "ctx.txt", context: 1 });
ok(withCtx.includes("ctx.txt-1-L1") && withCtx.includes("ctx.txt:2:"), "context lines carry the path too", withCtx);
ok(!withCtx.includes('"--"') && !/\\n--\\n/.test(withCtx), "and the bare -- separators are gone", withCtx);
ok((await call("grep", { pattern: "MARKER_CTX", glob: "**/dash-name.txt" })).includes("dash-name.txt"),
   "a glob matches a filename containing a dash");
// THE SEPARATOR ALSO OCCURS IN NAMES, so nothing in the line says where the path ends. An external
// audit measured `a-1-b.txt:1:MATCH` being read as `a`, because a non-greedy prefix stops at the
// `-1-` inside the NAME. Both files exist here on purpose: the answer has to be the LONGEST prefix
// that is a real file, or the shorter one wins.
fs.writeFileSync(path.join(ws, "a"), "MARKER_AMBIG\n");
fs.writeFileSync(path.join(ws, "a-1-b.txt"), "MARKER_AMBIG\n");
const ambig = await call("grep", { pattern: "MARKER_AMBIG" });
ok(ambig.includes("a-1-b.txt:1:") && ambig.includes("a:1:"), "both a and a-1-b.txt are named in full", ambig);
ok((await call("grep", { pattern: "MARKER_AMBIG", glob: "a-1-b.txt" })) .includes("a-1-b.txt"),
   "and a glob on the dashed name selects it");
ok(!(await call("grep", { pattern: "MARKER_AMBIG", glob: "a" })).includes("a-1-b.txt"),
   "while a glob on the short name does not drag it in");
// A matched line whose TEXT is exactly `--`: with -H it arrives as `path:N:--`, so the separator
// filter must not eat it.
fs.writeFileSync(path.join(ws, "dashline.txt"), "x\n--\n");
ok((await call("grep", { pattern: "^--$" })).includes("dashline.txt:2:--"), "a matched line that is exactly --");
// Four outcomes used to look identical because stderr was discarded: no matches, no grep in the
// image, an unsupported flag, a permission denial.
const badPattern = await call("grep", { pattern: "(" });
ok(badPattern.startsWith("THREW") && /grep/i.test(badPattern), "a pattern grep rejects says so instead of 'No matches'", badPattern);

console.log("\n== the other verbs, with the same arguments");
for (const [tool, params] of [
	["read", { path: "/etc/passwd" }],
	["read", { path: "link-out.txt" }],
	["ls", { path: "/etc" }],
	["find", { pattern: "*", path: "/etc" }],
	["write", { path: path.join(outside, "pwned.txt"), content: "x" }],
] as Array<[string, Record<string, unknown>]>) {
	ok(refused(await call(tool, params)), `${tool} refuses ${JSON.stringify(params.path ?? params)}`);
}
ok(!fs.existsSync(path.join(outside, "pwned.txt")), "and write left nothing outside the workspace");

console.log("\n== the positive control: the same verbs work inside");
ok((await call("read", { path: "proj.txt" })).includes("MARKER_PROJ"), "read inside");
ok((await call("ls", { path: "." })).includes("proj.txt"), "ls inside");
ok((await call("find", { pattern: "**/deep.txt" })).includes("deep.txt"), "find inside");

fs.rmSync(ws, { recursive: true, force: true });
fs.rmSync(outside, { recursive: true, force: true });
console.log(`\n  ${n - bad}/${n} PASS, ${bad} FAIL`);
process.exit(bad === 0 ? 0 : 1);
