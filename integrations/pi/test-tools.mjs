import activate from "kern-pi";
const T = {};
activate({ registerTool: t => (T[t.name] = t), registerCommand: () => {}, on: () => {} });
let bad = 0, n = 0;
const ok = (c, m, extra="") => { n++; console.log(`  ${c ? "PASS" : "FAIL"}  ${m}${extra && !c ? "  <- " + extra : ""}`); if (!c) bad++; };
const call = async (name, params) => {
  const out = [];
  try {
    const r = await T[name].execute("t" + n, params, new AbortController().signal, (u) => out.push(u), undefined);
    return { ok: true, r, text: JSON.stringify(r).slice(0, 400) };
  } catch (e) { return { ok: false, err: String(e).slice(0, 300) }; }
};

console.log("\n== il box e' vero, e non e' l'host");
let r = await call("bash", { command: "hostname; id -u; readlink /proc/1/exe || true" });
ok(r.ok, "bash esegue", r.err);
ok(r.ok && !r.text.includes(process.env.HOSTNAME ?? "@@nope@@"), "hostname non e' quello dell'host");
r = await call("bash", { command: "cat /proc/self/status | grep -E '^CapEff'" });
ok(r.ok && /CapEff:\s*0+$/m.test(JSON.stringify(r.r).replace(/\\n/g,"\n").replace(/\\t/g,"\t")), "CapEff a zero nel box", r.text);
r = await call("bash", { command: "cat /tmp/pix/OUTSIDE.txt 2>&1 || echo NOPE" });
ok(r.ok && !r.text.includes("segreto-host"), "un file host fuori dal workspace NON e' leggibile dal box", r.text);

console.log("\n== i sette tool, guidati davvero");
r = await call("write", { path: "made.txt", content: "scritto dal tool\n" });
ok(r.ok, "write", r.err);
r = await call("read", { path: "made.txt" });
ok(r.ok && r.text.includes("scritto dal tool"), "read rilegge cio' che write ha scritto", r.text);
r = await call("ls", { path: "." });
ok(r.ok && r.text.includes("sample.txt"), "ls elenca il workspace", r.text);
r = await call("grep", { pattern: "needle" });
ok(r.ok && r.text.includes("sample.txt"), "grep trova la stringa", r.text);
r = await call("find", { pattern: "**/deep.txt" });
ok(r.ok && r.text.includes("deep.txt"), "find trova il file annidato", r.text);
r = await call("edit", { path: "sample.txt", edits: [{ oldText: "alpha", newText: "ALPHA" }] });
ok(r.ok, "edit", r.err);
r = await call("read", { path: "sample.txt" });
ok(r.ok && r.text.includes("ALPHA"), "edit ha davvero cambiato il file", r.text);

console.log("\n== ostile: uscire dal workspace");
for (const p of ["/etc/passwd", "../OUTSIDE.txt", "../../etc/hostname", "/tmp/pix/OUTSIDE.txt"]) {
  const rr = await call("read", { path: p });
  const refused = !rr.ok || /refus|outside|denied|not allowed|workspace/i.test(rr.text ?? "");
  ok(refused, `read rifiuta '${p}'`, (rr.text ?? rr.err ?? "").slice(0,120));
}
const w = await call("write", { path: "/tmp/pix/PWNED.txt", content: "x" });
ok(!w.ok || /refus|outside|denied|workspace/i.test(w.text ?? ""), "write rifiuta un path assoluto fuori", (w.text??w.err).slice(0,120));

console.log("\n== rete spenta per default");
r = await call("bash", { command: "(curl -s -m 4 -o /dev/null -w '%{http_code}' https://example.com || echo NETFAIL) 2>&1" });
ok(r.ok && !/^"?200/.test(r.text.replace(/[^0-9A-Z]/g,"").slice(0,3)) , "il box non raggiunge internet", r.text);

console.log(`\n  ${n - bad}/${n} PASS, ${bad} FAIL`);
process.exit(bad === 0 ? 0 : 1);
