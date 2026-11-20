#!/usr/bin/env node
// The run's own event log, one line each: what happened, to which task, with what.
//
//   node events.mjs <run-id> [kind]
const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";
const [run, only] = process.argv.slice(2);
const page = await (await fetch(`${base}/`)).text();
const token = page.match(/name="devd-token" content="([^"]+)"/)[1];
const answer = await fetch(`${base}/api/invoke`, {
  method: "POST",
  headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
  body: JSON.stringify({ id: "team.get", args: { run } }),
});
const { outcome } = await answer.json();
for (const event of outcome.data.events ?? []) {
  if (only && event.kind !== only) continue;
  console.log(`${String(event.at).slice(11, 19)} ${event.kind.padEnd(14)} ${JSON.stringify(event.data).slice(0, 260)}`);
}
