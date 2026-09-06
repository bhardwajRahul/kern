/**
 * pi, for real, dispatching into this extension.
 *
 * Every other suite here calls the extension directly. This one runs the pi BINARY with the
 * extension loaded, lets pi's own tool dispatch invoke it, and reads the answer out of pi's JSON
 * event stream. Until it existed the claim "works with pi" rested on a type-check.
 *
 * The model is scripted (see e2e-provider.ts) because a model is what PRODUCES a tool call, and
 * that half tests the model. Everything after the call is pi's machinery and a real kern box.
 *
 *   node --experimental-strip-types test-e2e.ts
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// pi's `exports` map declares only the `import` condition, so `require.resolve` fails with
// ERR_PACKAGE_PATH_NOT_EXPORTED even though the package is right there. `import.meta.resolve`
// resolves under ESM conditions, which is the one that works.
let PI = "";
try {
	const entry = fileURLToPath(import.meta.resolve("@earendil-works/pi-coding-agent"));
	PI = path.join(entry.slice(0, entry.indexOf(`${path.sep}dist${path.sep}`)), "dist", "bundle", "cli.js");
} catch {
	console.log("  SKIP  @earendil-works/pi-coding-agent is not installed, so pi cannot be driven");
	process.exit(0);
}
if (!fs.existsSync(PI)) {
	console.log(`  SKIP  pi is installed but has no CLI bundle at ${PI}`);
	process.exit(0);
}

const here = path.dirname(new URL(import.meta.url).pathname);
const ws = fs.mkdtempSync(path.join(os.tmpdir(), "kern-pi-e2e-"));
let bad = 0;
let n = 0;
function ok(cond: boolean, what: string, saw?: unknown): void {
	n++;
	console.log(`  ${cond ? "PASS" : "FAIL"}  ${what}${cond ? "" : `  <- ${JSON.stringify(saw)?.slice(0, 220)}`}`);
	if (!cond) bad++;
}

/** Run pi once with a scripted list of tool calls, and return its parsed event stream. */
function drive(script: Array<{ name: string; arguments: Record<string, unknown> }>): any[] {
	const r = spawnSync(
		process.execPath,
		[PI, "-p", "go", "--mode", "json", "--model", "kern-e2e/scripted",
		 "-e", path.join(here, "e2e-provider.ts"), "-e", path.join(here, "index.ts")],
		{ cwd: ws, encoding: "utf8", timeout: 300_000,
		  env: { ...process.env, KERN_E2E_SCRIPT: JSON.stringify(script) } },
	);
	return (r.stdout ?? "").split("\n").flatMap((l) => { try { return [JSON.parse(l)]; } catch { return []; } });
}
const endOf = (events: any[], tool: string) =>
	events.find((e) => e.type === "tool_execution_end" && e.toolName === tool);
const textOf = (ev: any) => (ev?.result?.content ?? []).map((c: any) => c.text ?? "").join("");

console.log("\n== pi dispatches bash into a real box");
let ev = drive([{ name: "bash", arguments: { command: "echo E2E-MARKER; id -u; hostname; ls /" } }]);
ok(ev.length > 0, "pi produced an event stream");
const bash = endOf(ev, "bash");
ok(!!bash, "pi reports tool_execution_end for bash", ev.map((e) => e.type));
const out = textOf(bash);
ok(out.includes("E2E-MARKER"), "the command ran and its stdout came back through pi", out);
ok(!out.includes(os.hostname()), "the hostname is the box's, not this host's", out);
ok(/^0$/m.test(out), "uid 0 inside the box, while this process is not root on the host", out);
// The discriminator is /workspace: the box has it because the SDK mounts the cwd there, and the
// host root does not. Written first as "the listing has no /home", which the python image has and
// the assertion had no business caring about.
ok(/^workspace$/m.test(out) && !fs.existsSync("/workspace"), "the listing is the BOX's root: it has /workspace and this host does not", out);

console.log("\n== and the file verbs, through pi as well");
ev = drive([
	{ name: "write", arguments: { path: "note.txt", content: "written through pi\n" } },
	{ name: "read", arguments: { path: "note.txt" } },
]);
ok(!!endOf(ev, "write"), "pi reports tool_execution_end for write");
ok(textOf(endOf(ev, "read")).includes("written through pi"), "read returns what write wrote", textOf(endOf(ev, "read")));
ok(fs.existsSync(path.join(ws, "note.txt")), "and it landed in the host workspace, which is the point of the mount");

console.log("\n== the gate holds when the call comes from pi, not from a test");
ev = drive([{ name: "read", arguments: { path: "/etc/passwd" } }]);
const denied = endOf(ev, "read");
ok(
	denied?.isError === true || /refus|outside|workspace/i.test(textOf(denied)),
	"reading outside the workspace is refused",
	textOf(denied),
);

fs.rmSync(ws, { recursive: true, force: true });
console.log(`\n  ${n - bad}/${n} PASS, ${bad} FAIL`);
process.exit(bad === 0 ? 0 : 1);
