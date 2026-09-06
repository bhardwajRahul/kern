/**
 * The half of this extension that needs no pi.
 *
 * Split out so an auditor can import it from the published tarball and exercise the path gate,
 * the box options and the glob matcher WITHOUT installing the peer. This code used to sit in
 * index.ts, which imports pi at the top for the seven tool factories, so importing any of it
 * threw `Cannot find package '@earendil-works/pi-coding-agent'`. Raised in an external audit of
 * 0.1.1, and it is why the path gate had no test a stranger could run.
 */
import path from "node:path";
import fs from "node:fs";
import os from "node:os";
import { DEFAULT_TMPFS_MB, Sandbox, type SandboxOptions } from "kern-sandbox";

/** Where the kern SDK mounts the workspace inside every box (`kern-sandbox`'s own constant). */
export const GUEST_WORKSPACE = "/workspace";

/** Where the box's `$HOME` points. Every modern toolchain caches under it, and in a read-only root
 * that cache cannot be created. What the user sees is NOT a permission error: npm renders a failed
 * `mkdir /root/.npm` as `Invalid response body while trying to fetch https://registry.npmjs.org/...`,
 * which sends them to check egress, which was already correct. Measured on `node:22`, and both halves
 * are load-bearing: with neither, `npm install express` exits 2; with `HOME` alone and no writable
 * `/tmp`, it still exits 2; with both, it exits 0.
 *
 * It points at the WORKSPACE, and a reviewer argued it should point at the scratch instead: a cache
 * is not a project artifact, and `npm install express` leaves **7.7 MB in `/workspace/.npm` on the
 * host**, where nothing bounds it, plus a `.npm` directory in the user's project. Both facts are
 * true and measured. The recommendation was still tried and REFUTED, by measuring the premise
 * underneath it rather than the recommendation itself:
 *
 *     command 1: mkdir -p /tmp/home && echo > /tmp/home/marker   -> SCRITTO
 *     command 2: cat /tmp/home/marker                            -> SPARITO
 *     control:   the same two commands against /workspace        -> survives
 *
 * Every command is a FRESH BOX, so the scratch is fresh too. `HOME` on the scratch means `$HOME` does
 * not exist when a command starts and the package cache is rebuilt from the network on EVERY command,
 * which is worse for the agent this extension exists to serve than an unbounded cache is. The only
 * persistent writable path is the workspace, so with per-command boxes, persistence and boundedness
 * cannot both hold here. `KERN_PI_HOME=/tmp/home` remains available for anyone who wants the
 * ephemeral, bounded side of that trade. */
export const HOME = process.env.KERN_PI_HOME ?? GUEST_WORKSPACE;

/** Knobs, all optional. Deliberately env vars rather than a config file: an extension a user drops in
 * with `pi -e` should need nothing else, and the two that matter are the image and egress. */
export const IMAGE = process.env.KERN_PI_IMAGE ?? "python:3.12-slim";

export const MEMORY_MB = intFromEnv("KERN_PI_MEMORY_MB", 2048);

export const PIDS = intFromEnv("KERN_PI_PIDS", 512);

export const TIMEOUT_S = intFromEnv("KERN_PI_TIMEOUT", 120);

/** Cap on captured stdout and stderr, EACH, per command. The SDK's own default is 64 MiB, and this
 * lowers it by two orders of magnitude on purpose: `onData` forwards every chunk into pi's renderer
 * as it arrives, pi is single-threaded, and the agent picks the command. `yes` or
 * `cat /dev/urandom` is the same premise as the glob that took 149 seconds, with the output as the
 * lever instead of the pattern. pi truncates tool output for the model anyway, so the bytes above
 * this are spent rendering something nobody reads. Measured: the timeout does kill the process in the
 * box rather than merely stop reading it, so the cap bounds the burst and the deadline ends it. */
export const MAX_OUTPUT = intFromEnv("KERN_PI_MAX_OUTPUT", 1024 * 1024);

/** Comma-separated hosts the box may reach, e.g. "registry.npmjs.org,pypi.org". Empty = no network.
 * NOT a boolean: `network: true` would share the host's whole network on every command, and an agent
 * that needs one registry does not need that. */
export const EGRESS = (process.env.KERN_PI_EGRESS ?? "")
	.split(",")
	.map((s) => s.trim())
	.filter(Boolean);

/** Scratch at `/tmp`, in MiB. The box root is read-only, so a toolchain gets exactly two writable
 * places: the workspace, and this. 256 rather than the SDK's 64 because the agent here installs
 * things: `npm install express` on `node:22` fits in 64 MiB, a real dependency tree does not, and the
 * failure it produces is not legible (see `HOME` below). It is a MULTIPLE of the SDK's own default
 * rather than a second independent number, so the two cannot drift apart: if the SDK moves, this
 * moves with it and the 4x relationship stays the thing that was decided. It is charged to the box's
 * own memory cgroup,
 * so it costs nothing until it is used and overrunning it is the box's OOM, never the host's disk.
 * `0` means none at all, the same sentinel `KERN_MCP_TMPFS_MB` uses. */
export const TMPFS_MB = scratchFromEnv("KERN_PI_TMPFS_MB", DEFAULT_TMPFS_MB * 4);

/**
 * THE SIDE THAT REFUSED IS PART OF THE MESSAGE.
 *
 * Six verbs fail with box-side errors and two with host-side ones, and until now the agent saw both
 * and could not tell which spoke. That is the same defect as kern's `--memory` hint and the `126`
 * message: a reader sent to the wrong place. `ENOENT: /workspace/x` says nothing about whether the
 * box could not find it, the workspace refused it, or this extension declined to look.
 *
 * Three sides, and every throw in this file names one:
 *   `kern[gate]` this extension refused before anything ran, from the path alone
 *   `kern[box]`  a command in the box answered, or the box could not do it
 *   `kern[host]` the SDK's host-side guard refused, or a host syscall failed
 *
 * pi renders the message to the model, so the prefix is the only channel that survives to the reader.
 */
export type Side = "gate" | "box" | "host";

/** Which shell the box actually has. pi's tool is called `bash` and a model writes bash by reflex, so
 * bash is what we want; `sh` is the fallback that every image has. */
export type Shell = "bash" | "sh";

export function intFromEnv(name: string, fallback: number): number {
	const raw = process.env[name];
	if (raw === undefined) return fallback;
	const n = Number.parseInt(raw, 10);
	return Number.isFinite(n) && n > 0 ? n : fallback;
}

export function refuse(side: Side, message: string): Error {
	return new Error(`kern[${side}]: ${message}`);
}

/** Segment-wise, so `**` is the only thing that can consume a `/`. Recursion depth is bounded by the
 * number of `**` in the pattern, not by the length of either string. */
/** One segment against one segment. `*` and `?` here cannot cross a separator because neither string
 * contains one. Two pointers, one remembered star: no recursion, no backtracking blow-up. */
export function matchOneSegment(pat: string, s: string): boolean {
	let p = 0;
	let i = 0;
	let starP = -1;
	let starI = 0;
	while (i < s.length) {
		const c = pat[p];
		if (p < pat.length && (c === "?" || c === s[i])) {
			p++;
			i++;
		} else if (p < pat.length && c === "*") {
			starP = p++;
			starI = i;
		} else if (starP >= 0) {
			// The last star eats one more character. This is the ONLY backtrack, and it advances, so
			// the loop runs at most once per position of `s`.
			p = starP + 1;
			i = ++starI;
		} else {
			return false;
		}
	}
	while (p < pat.length && pat[p] === "*") p++;
	return p === pat.length;
}

export function matchSegments(pat: string[], seg: string[], pi: number, si: number): boolean {
	while (pi < pat.length) {
		if (pat[pi] === "**") {
			// `**` matches zero or more segments. Try the shortest first so the common case where it
			// stands at the end returns immediately.
			for (let skip = si; skip <= seg.length; skip++) {
				if (matchSegments(pat, seg, pi + 1, skip)) return true;
			}
			return false;
		}
		if (si >= seg.length) return false;
		if (!matchOneSegment(pat[pi], seg[si])) return false;
		pi++;
		si++;
	}
	return si === seg.length;
}

/** Like `intFromEnv`, plus `0` meaning "none at all" rather than falling back to the default.
 *
 * `KERN_MCP_TMPFS_MB=0` already means that in the MCP server, and two knobs with the same name and
 * opposite behaviour is the defect this whole round has been about: an operator who typed 0 got 256
 * and nothing said so. Garbage and negatives still fall back, because those are typos rather than
 * decisions. */
export function scratchFromEnv(name: string, fallback: number): number {
	const raw = process.env[name];
	if (raw !== undefined && raw.trim() === "0") return 0;
	return intFromEnv(name, fallback);
}

/** `$HOME` must be an ABSOLUTE path in the box. An empty string turns `$HOME/.npm` into `/.npm`,
 * which is inside the read-only root, so the defect this knob exists to fix comes straight back with
 * no message; a relative one resolves against whatever the command's cwd happens to be. Both are
 * refused here rather than discovered as an npm error about the network. */
export function guestHome(home: string = HOME): string {
	if (!home.startsWith("/")) {
		throw refuse(
			"gate",
			`KERN_PI_HOME must be an absolute path in the box, got ${JSON.stringify(home)}. ` +
				"An empty or relative value puts the toolchain cache back inside the read-only root.",
		);
	}
	return home;
}

/** An SDK too old to honour `tmpfs` would IGNORE it: an unknown constructor option is not an error in
 * either binding, so the box would come up with a read-only `/tmp` and the first `npm install` would
 * fail with a message about the network. Ask the installed SDK what it did with the option rather
 * than what version it says it is, and fail here, once, naming the fix.
 *
 * Exported so a test can call it: the whole point is that it fires before a box exists. */
export function requireScratchSupport(): void {
	const probe = new Sandbox({ tmpfs: {} }) as unknown as { tmpfs?: unknown };
	if (probe.tmpfs === undefined) {
		throw refuse(
			"host",
			"the installed kern-sandbox ignores `tmpfs`, so the box would get a read-only /tmp and a " +
				"toolchain would fail with a message about the network. Upgrade: npm install kern-sandbox@^0.1.36",
		);
	}
}

/** Every option the box is opened with, in one place a test can read without starting one.
 *
 * It lives here and not inline in `ensureBox` because the two lines that matter most are the two that
 * were MISSING: with no `tmpfs` and no `HOME`, the box has exactly one writable path, and every
 * toolchain that caches (npm, Go, .NET, Maven, pip's wheel cache) fails somewhere far from the cause.
 * A default nobody can see is a default nobody checks. */
export function boxOptions(hostWorkspace: string): SandboxOptions {
	return {
		image: IMAGE,
		workspace: hostWorkspace, // NOT deleted on close: the SDK only removes what it created
		memoryMb: MEMORY_MB,
		pids: PIDS,
		timeoutS: TIMEOUT_S,
		maxOutputBytes: MAX_OUTPUT,
		...(TMPFS_MB > 0 ? { tmpfs: { "/tmp": `${TMPFS_MB}m` } } : { tmpfs: {} }),
		env: { HOME: guestHome() },
		...(EGRESS.length > 0 ? { egressAllow: EGRESS } : {}),
		trackFiles: false, // pi reports its own file changes; skip the per-call workspace diff
	};
}

/**
 * THE CONTAINMENT CHECK. Every path from pi passes through here, and nothing else in this file
 * touches the filesystem without it.
 *
 * pi hands us absolute GUEST paths because each tool is built with `GUEST_WORKSPACE` as its cwd, so
 * `/workspace/src/main.rs` arrives and `src/main.rs` is what the SDK and the host both want.
 *
 * Refused, rather than clamped or remapped:
 *   - anything not under /workspace (`/etc/passwd`, `/workspace/../etc/passwd`)
 *   - a relative path, which would mean pi's cwd is not what we configured and the caller and this
 *     function disagree about the frame of reference. Guessing which one is right is how an escape
 *     gets written.
 *
 * `path.posix.resolve` collapses `..` BEFORE the prefix test, so `/workspace/../etc` is normalised to
 * `/etc` and then refused. Testing the raw string would pass it.
 */
export function refuseOutsideWorkspace(absolutePath: string): string {
	// A GATE MUST REFUSE, NOT CRASH. The argument arrives from a model, so `null`, a number or a
	// missing field are reachable inputs, and `.trim()` on them threw a TypeError that reads like a
	// bug in the extension rather than a rejected path. Raised in an external audit of 0.1.1.
	if (typeof absolutePath !== "string") {
		throw refuse("gate", `refusing a path that is not a string: ${typeof absolutePath}`);
	}
	// A NUL cannot appear in a real filename and truncates the path in every syscall that takes a
	// C string, so a name is one thing to this gate and another to the kernel. Newlines are legal in
	// POSIX filenames and are NOT refused: the shell path quotes with single quotes, and refusing
	// them would refuse files that exist.
	if (absolutePath.includes("\0")) {
		throw refuse("gate", "refusing a path containing a NUL byte");
	}
	const raw = absolutePath.trim();
	if (!path.posix.isAbsolute(raw)) {
		throw refuse("gate", `refusing a relative path from the agent: ${absolutePath}`);
	}
	const resolved = path.posix.resolve(raw);
	if (resolved === GUEST_WORKSPACE) return "";
	if (!resolved.startsWith(`${GUEST_WORKSPACE}/`)) {
		throw refuse(
			"gate",
			`refusing a path outside ${GUEST_WORKSPACE}: ${absolutePath}\n` +
				`The box cannot see it and neither will these tools. Start pi from the directory you want the agent to work in.`,
		);
	}
	return resolved.slice(GUEST_WORKSPACE.length + 1);
}

/**
 * The MIME type of an image, from its MAGIC BYTES rather than its extension.
 *
 * pi's read tool needs this to hand an image to the model as an image instead of as bytes, and it is
 * optional in the interface, so leaving it out silently downgrades every screenshot in the project to
 * garbage text. Sniffing beats trusting the name: a `.png` that is really a JPEG would be described
 * wrongly to the model, and an agent renaming a file cannot change what it is.
 *
 * Only the four formats pi itself resizes. Anything else returns null, which is the interface's way
 * of saying "not an image", and the read tool falls back to its normal path.
 */
export async function detectImageMimeType(ws: string, rel: string): Promise<string | null> {
	// NOT `box.readFile(rel, { maxBytes: 16 })`. In the SDK `maxBytes` is a REFUSAL threshold, not a
	// partial read: a file larger than it throws rather than returning its head. The first version
	// here did exactly that and the catch below swallowed the error, so every image over 16 bytes
	// reported "not an image" in silence. A bounded host read is what a sniff wants, and reading a
	// whole 5 MB PNG to look at eight bytes is what the alternative costs.
	//
	// The containment the SDK would have applied is re-applied here rather than assumed: the caller
	// has already run [[refuseOutsideWorkspace]] on the guest path, which stops `..`, and realpath
	// plus a prefix test stops a symlinked DIRECTORY component, which a path check alone does not.
	// O_NOFOLLOW then stops the final component being a link.
	let head: Buffer;
	try {
		const wsReal = fs.realpathSync(ws);
		const target = fs.realpathSync(path.join(wsReal, rel));
		// Outside the workspace: not an image we are allowed to look at, which IS "not an image"
		// as far as every tool here is concerned. Not a swallowed failure.
		if (target !== wsReal && !target.startsWith(wsReal + path.sep)) return null;
		const fd = fs.openSync(target, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK);
		try {
			// POST-OPEN, and this is the last host-side surface in the file. `realpath` then open-by-path
			// is resolve-then-use: between the two, a component of `target` can become a symlink and the
			// open follows it. `O_NOFOLLOW` covers the leaf only. Asking the KERNEL where the descriptor
			// landed closes it, because the fd is bound at open time and cannot be redirected afterwards.
			//
			// It is the SDK's own construction, and measured independently sufficient: with the SDK's
			// pre-walk disabled and this check alone, 7,684 concurrent swaps produced 598,321 refusals
			// and zero host bytes. Copied rather than invented for that reason.
			const landed = fs.readlinkSync(`/proc/self/fd/${fd}`);
			if (landed !== wsReal && !landed.startsWith(wsReal + path.sep)) return null;
			head = Buffer.alloc(16);
			const n = fs.readSync(fd, head, 0, 16, 0);
			head = head.subarray(0, n);
		} finally {
			fs.closeSync(fd);
		}
	} catch (e) {
		// NARROW, because `null` here means "not an image" and the caller cannot tell that apart from
		// "we could not look". Swallowing everything is how `maxBytes` produced a valid-looking answer
		// for every screenshot in a project: a catch may turn an error into a DIFFERENT error, never
		// into a value indistinguishable from success.
		//
		// The three below are cases where "not an image" is the TRUE answer, so null is not a guess:
		// the path is gone, it is a directory, or a component of it is not one. Everything else
		// (EACCES, EIO, a bad descriptor) is a failure to look, and the read tool that is about to
		// open the same file should hear about it now rather than be told there is no picture.
		const code = (e as NodeJS.ErrnoException)?.code;
		if (code === "ENOENT" || code === "EISDIR" || code === "ENOTDIR" || code === "ELOOP") return null;
		throw e;
	}
	if (head.length >= 8 && head.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))) {
		return "image/png";
	}
	if (head.length >= 3 && head[0] === 0xff && head[1] === 0xd8 && head[2] === 0xff) return "image/jpeg";
	if (head.length >= 6 && head.subarray(0, 6).toString("latin1").match(/^GIF8[79]a$/)) return "image/gif";
	if (
		head.length >= 12 &&
		head.subarray(0, 4).toString("latin1") === "RIFF" &&
		head.subarray(8, 12).toString("latin1") === "WEBP"
	) {
		return "image/webp";
	}
	return null;
}

/**
 * Does `candidate` (a workspace-relative POSIX path) match `glob`?
 *
 * NOT a RegExp, and the reason is measured. The first version compiled the glob into an anchored
 * regex, which turns `a*` repeated sixty times into `(a[^/]*){60}b` and makes a match against four
 * hundred `a`s take **149 SECONDS** of catastrophic backtracking. The glob comes from the agent, node
 * is single-threaded, and pi's whole session shares this event loop: that is a denial of service the
 * agent chooses, not a slow function.
 *
 * This is the classic two-pointer wildcard match instead, which never backtracks more than once per
 * star and is O(n*m) in the worst case rather than exponential. Semantics unchanged:
 *
 *   `**`  crosses `/` (matches whole path segments, including none)
 *   `*`   does not cross `/`
 *   `?`   exactly one non-`/` character
 *   everything else is literal, including every regex metacharacter
 */
export function globMatches(glob: string, candidate: string): boolean {
	return matchSegments(glob.split("/"), candidate.split("/"), 0, 0);
}

/** Ask the box which shell it has, once, at open. Not a guess from the image tag: `python:3.12-slim`
 * carries bash, alpine does not, and the tag says neither. Costs one box (~90 ms) per session.
 *
 * This exists because the SDK's `language: "bash"` used to run `sh`, which on a Debian image is dash:
 * the agent's `[[ -f x ]]` answered `sh: 1: [[: not found` with bash sitting unused in the same image.
 * Now that `bash` means bash, asking for it on an image without one would fail EVERY command, so the
 * choice has to be measured rather than hardcoded either way. */
export async function detectShell(box: Sandbox): Promise<Shell> {
	try {
		const r = await box.runCode('command -v bash >/dev/null 2>&1 && echo bash || echo sh', { language: "sh" });
		return r.stdout.trim().endsWith("bash") ? "bash" : "sh";
	} catch {
		return "sh"; // a probe that cannot run is not a reason to pick the shell that may not exist
	}
}
