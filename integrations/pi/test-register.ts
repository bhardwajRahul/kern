// Does the extension REGISTER with pi at all? The three suites here drive the half that talks to
// kern; nothing had ever called the default export, which is the half a user meets first.
import activate from "./index.ts";

const tools: string[] = [];
const commands: string[] = [];
const events: string[] = [];
const api = {
  registerTool: (t: any) => { tools.push(t?.name ?? "<senza nome>"); },
  registerCommand: (n: string, _s: any) => { commands.push(n); },
  on: (e: string, _h: any) => { events.push(e); },
} as any;

activate(api);

const wantTools = ["bash", "read", "write", "edit", "ls", "grep", "find"];
let bad = 0;
const ok = (c: boolean, m: string) => { console.log(`  ${c ? "PASS" : "FAIL"}  ${m}`); if (!c) bad++; };

ok(tools.length === 7, `registra 7 tool (ne ha registrati ${tools.length}: ${tools.join(", ")})`);
for (const w of wantTools) ok(tools.includes(w), `sostituisce il tool '${w}' di pi`);
ok(commands.includes("kern"), `registra il comando /kern (${commands.join(", ") || "nessuno"})`);
ok(events.length === 2, `si aggancia a 2 eventi (${events.join(", ")})`);
console.log(bad === 0 ? "\n  registrazione: OK" : `\n  registrazione: ${bad} FALLITE`);
process.exit(bad === 0 ? 0 : 1);
