#!/usr/bin/env node
// Why a run ended: its error, its verdict, and the report of the task that judged it.
//
//   node runwhy.mjs <run-id>
const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";
const page = await (await fetch(`${base}/`)).text();
const token = page.match(/name="devd-token" content="([^"]+)"/)[1];
const answer = await fetch(`${base}/api/invoke`, {
  method: "POST",
  headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
  body: JSON.stringify({ id: "team.get", args: { run: process.argv[2] } }),
});
const { outcome } = await answer.json();
const data = outcome.data;
console.log(`state: ${data.run.state} · verdict: ${JSON.stringify(data.run.verdict)}`);
console.log(`error: ${data.run.error ?? "(none)"}`);
for (const task of data.tasks ?? []) {
  if (!task.result) continue;
  console.log(`\n--- ${task.key} (${task.role}) ${task.state}`);
  console.log(String(task.result).slice(0, 3000));
}
