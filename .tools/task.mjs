#!/usr/bin/env node
// Print one task of a run in full: state, error, criteria, verdict and report.
//
//   node task.mjs <run-id> <key> [width]
const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";
const [run, key, width = "4000"] = process.argv.slice(2);
const page = await (await fetch(`${base}/`)).text();
const token = page.match(/name="devd-token" content="([^"]+)"/)[1];
const answer = await fetch(`${base}/api/invoke`, {
  method: "POST",
  headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
  body: JSON.stringify({ id: "team.get", args: { run } }),
});
const { outcome } = await answer.json();
const task = (outcome.data.tasks ?? []).find((t) => t.key === key);
if (!task) {
  console.error(`no task ${key}`);
  process.exit(1);
}
const cut = (text) => String(text ?? "").slice(0, Number(width));
console.log(`task ${task.key} (${task.role}) ${task.state} attempts=${task.attempts}`);
console.log(`workdir: ${task.workdir} branch: ${task.branch ?? "-"}`);
if (task.error) console.log(`error: ${cut(task.error)}`);
if (task.verdict) console.log(`verdict: ${JSON.stringify(task.verdict).slice(0, 300)}`);
if (task.result) console.log(`\nreport:\n${cut(task.result)}`);
if (task.criteria?.length) {
  console.log(`\ncriteria:`);
  for (const criterion of task.criteria) console.log(`  [${criterion.status ?? "-"}] ${criterion.text ?? criterion}`);
}
if (task.checks?.length) console.log(`checks: ${JSON.stringify(task.checks).slice(0, 600)}`);
if (task.evidence?.length) {
  console.log(`\nevidence (${task.evidence.length}):`);
  for (const item of task.evidence.slice(0, 12)) console.log(`  ${item.kind} ${item.capability ?? ""} ok=${item.ok} ${String(item.summary ?? "").slice(0, 140)}`);
}
