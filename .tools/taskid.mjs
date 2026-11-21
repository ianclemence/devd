#!/usr/bin/env node
// Print the id of a run's task by its key, for the capabilities that want an id.
//
//   node taskid.mjs <run-id> <key>
const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";
const [run, key] = process.argv.slice(2);
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
process.stdout.write(task.id + "\n");
