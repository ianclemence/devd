#!/usr/bin/env node
// One line per task of a run, plus its state and what it is waiting for.
//
//   node runstate.mjs <run-id>
const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";

const page = await (await fetch(`${base}/`)).text();
const token = page.match(/name="devd-token" content="([^"]+)"/)[1];
const answer = await fetch(`${base}/api/invoke`, {
  method: "POST",
  headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
  body: JSON.stringify({ id: "team.get", args: { run: process.argv[2] } }),
});
const { outcome } = await answer.json();
if (!outcome.ok) {
  console.error(outcome.message);
  process.exit(1);
}
const data = outcome.data;
console.log(`run ${data.run.id} · ${data.run.state} · driver ${data.driver} · $${data.usage?.cost_usd?.toFixed(4) ?? "?"} · ${data.usage?.turns ?? "?"} turns`);
for (const task of data.tasks ?? []) {
  console.log(`  ${task.key.padEnd(15)} ${task.state.padEnd(10)} attempts=${task.attempts} ${task.role.padEnd(12)} ${task.error ? "· " + String(task.error).slice(0, 120) : ""}`);
}
if (data.needs) console.log(`needs: ${data.needs.reason} · ${data.needs.line ?? ""} · ${(data.needs.missing ?? []).join("; ")}`);
if (data.run.verdict) console.log(`verdict: ${JSON.stringify(data.run.verdict).slice(0, 400)}`);
if (data.run.summary) console.log(`summary: ${String(data.run.summary).replace(/\s+/g, " ").slice(0, 1200)}`);
if (data.run.error) console.log(`error: ${data.run.error}`);
