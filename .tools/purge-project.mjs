#!/usr/bin/env node
// Find, export and (optionally) delete everything devd still holds for one project.
//
//   node purge-project.mjs <substring>            list and export
//   node purge-project.mjs <substring> --delete   list, export, then delete
//
// "Everything" is: the team runs whose project matches, and every conversation whose project
// matches or whose team run is one of those. Exports land beside the archive so the record
// survives the deletion.
import { mkdirSync, writeFileSync } from "node:fs";

const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";
const needle = (process.argv[2] ?? "").toLowerCase();
const remove = process.argv.includes("--delete");
if (!needle) {
  console.error("usage: node purge-project.mjs <substring> [--delete]");
  process.exit(2);
}

const page = await (await fetch(`${base}/`)).text();
const token = page.match(/name="devd-token" content="([^"]+)"/)[1];
const invoke = async (id, args) => {
  const answer = await fetch(`${base}/api/invoke`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ id, args }),
  });
  const body = await answer.json();
  return body;
};

// A destructive capability answers with a token instead of acting; the token is the person's
// decision, and /api/approve applies exactly the call that was described.
const approve = async (body) => {
  if (!body?.token) return body;
  const answer = await fetch(`${base}/api/approve`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
    body: JSON.stringify({ token: body.token }),
  });
  return answer.json();
};

const runs = (await invoke("team.list", {})).outcome?.data ?? [];
const cadenceRuns = runs.filter((r) => String(r.project ?? "").toLowerCase().includes(needle));
console.log(`runs matching "${needle}":`);
for (const r of cadenceRuns) console.log(`  ${r.id}  ${r.state.padEnd(10)} ${String(r.project ?? "")}`);

const threads = (await invoke("chat.list", {})).outcome?.data ?? [];
const cadenceThreads = threads.filter(
  (t) =>
    String(t.project ?? "").toLowerCase().includes(needle) ||
    cadenceRuns.some((r) => t.team?.run === r.id),
);
console.log(`conversations matching "${needle}": ${cadenceThreads.length}`);
for (const t of cadenceThreads) console.log(`  ${t.id}  ${t.state.padEnd(8)} ${t.title.slice(0, 70)}`);

const out = `D:\\devd\\archive\\${needle}`;
mkdirSync(`${out}\\threads`, { recursive: true });
for (const t of cadenceThreads) {
  const exported = await invoke("chat.export", { thread: t.id });
  writeFileSync(`${out}\\threads\\${t.id}.json`, JSON.stringify(exported, null, 2));
}
for (const r of cadenceRuns) {
  const got = await invoke("team.get", { run: r.id });
  writeFileSync(`${out}\\runs\\${r.id}.json`, JSON.stringify(got, null, 2));
}
console.log(`exported to ${out}`);

if (!remove) {
  console.log("\n(dry run: pass --delete to remove them)");
  process.exit(0);
}

for (const r of cadenceRuns) {
  if (r.state === "running" || r.state === "paused" || r.state === "planning") {
    const cancelled = await invoke("team.cancel", { run: r.id });
    console.log(`cancel ${r.id}: ${cancelled.outcome?.message ?? JSON.stringify(cancelled).slice(0, 200)}`);
  }
  const deleted = await approve(await invoke("team.delete", { run: r.id }));
  console.log(`delete run ${r.id}: ${deleted.outcome?.message ?? JSON.stringify(deleted).slice(0, 300)}`);
}
for (const t of cadenceThreads) {
  const deleted = await approve(await invoke("chat.delete", { thread: t.id }));
  console.log(`delete conversation ${t.id}: ${deleted.outcome?.message ?? JSON.stringify(deleted).slice(0, 300)}`);
}
const forgotten = await invoke("project.forget", { path: `D:\\devd\\www\\welcome\\${needle}` });
console.log(`forget project: ${forgotten.outcome?.message ?? JSON.stringify(forgotten).slice(0, 300)}`);
