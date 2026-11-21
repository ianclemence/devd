#!/usr/bin/env node
// Follow a team run and write down everything it does, from devd's own record.
//
//   node monitor.mjs <run-id> [--every 5] [--out run.log]
//
// Every tick reads team.get and chat.team_log. Task state changes, plan, messages, tool calls and
// their results are appended with a timestamp, so the log is the run's history in the order it
// happened rather than a reconstruction. Stops when the run reaches a terminal state.
import { appendFileSync, writeFileSync } from "node:fs";

const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";
const args = process.argv.slice(2);
const run = args[0];
if (!run) {
  console.error("usage: monitor.mjs <run-id> [--every 5] [--out run.log]");
  process.exit(2);
}
const flag = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const every = Number(flag("every", "5")) * 1000;
const out = flag("out", `D:\\devd\\www\\devd\\.tools\\run-${run.slice(0, 8)}.log`);

async function token() {
  if (process.env.DEVD_TOKEN) return process.env.DEVD_TOKEN;
  const page = await (await fetch(`${base}/`)).text();
  return page.match(/name="devd-token" content="([^"]+)"/)[1];
}
const bearer = await token();

async function invoke(id, input) {
  const answer = await fetch(`${base}/api/invoke`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${bearer}` },
    body: JSON.stringify({ id, args: input }),
  });
  const body = await answer.json();
  if (!body.outcome?.ok) throw new Error(`${id}: ${body.outcome?.message ?? answer.status}`);
  return body.outcome.data;
}

const line = (text) => {
  const stamped = `${new Date().toISOString()} ${text}`;
  appendFileSync(out, stamped + "\n");
  console.log(stamped);
};

writeFileSync(out, "");
line(`monitor: run ${run}, every ${every / 1000}s`);

const tasks = new Map();
let origin = null;
let lastEvent = 0;
let seenEvents = 0;

for (;;) {
  const snapshot = await invoke("team.get", { run });
  const state = snapshot.run.state;
  const goal = snapshot.run.title;
  if (!seenEvents) {
    line(`goal: ${goal}`);
    line(`state: ${state}  project: ${snapshot.run.project ?? "-"}`);
    if (snapshot.budget) line(`budget: ${JSON.stringify(snapshot.budget)}`);
  }
  for (const task of snapshot.tasks ?? []) {
    const before = tasks.get(task.key);
    if (!before) {
      tasks.set(task.key, task.state);
      line(`task + ${task.key} [${task.role}] "${task.title}" ${task.state}${task.depends_on?.length ? " after " + task.depends_on.join(",") : ""}`);
    } else if (before !== task.state) {
      tasks.set(task.key, task.state);
      line(`task ~ ${task.key} [${task.role}] ${before} -> ${task.state}${task.error ? " · " + String(task.error).slice(0, 160) : ""}`);
    }
  }
  if (snapshot.needs) line(`needs: ${JSON.stringify(snapshot.needs).slice(0, 400)}`);

  if (!origin) {
    // A run started without a conversation has no room of its own. `chat.team_log` read from any of
    // its members still returns every thread the run has, which is what monitor needs.
    const any = (snapshot.tasks ?? []).map((t) => t.thread_id).find((id) => id);
    if (any) {
      origin = (await invoke("chat.team_log", { thread: any, limit: 1 })).origin;
      origin = origin ?? any;
    }
  }
  if (origin) {
    const log = await invoke("chat.team_log", { thread: origin, limit: 500 });
    const fresh = (log.events ?? []).filter((e) => e.id > lastEvent);
    for (const event of fresh) {
      lastEvent = Math.max(lastEvent, event.id);
      const who = event.role ?? (event.origin ? "devd" : "person");
      const data = event.data ?? {};
      if (event.kind === "assistant") {
        for (const block of data.content ?? []) {
          if (block.type === "text") line(`${who} says: ${String(block.text).replace(/\s+/g, " ").slice(0, 400)}`);
          else if (block.type === "tool_use") line(`${who} calls ${block.name} ${JSON.stringify(block.input).slice(0, 240)}`);
        }
      } else if (event.kind === "tool_result") {
        const body = String(data.content ?? "").replace(/\s+/g, " ");
        line(`  result ${data.tool_use_id ?? ""}: ${body.slice(0, 240)}`);
      } else if (event.kind === "user") {
        line(`person says: ${String(data.text ?? "").replace(/\s+/g, " ").slice(0, 300)}`);
      } else if (event.kind === "agent") {
        line(`${who} -> @${data.to ?? "room"} about ${data.about ?? "-"}: ${String(data.text ?? "").slice(0, 240)}`);
      } else if (event.kind === "changes") {
        line(`  changes: ${(data.files ?? []).map((f) => `${f.status} ${f.path}`).join(", ").slice(0, 240)}`);
      } else {
        line(`${event.kind}: ${JSON.stringify(data).slice(0, 200)}`);
      }
      seenEvents += 1;
    }
  }

  if (["done", "failed", "cancelled"].includes(state)) {
    line(`state: ${state} — final`);
    const usage = snapshot.usage ? JSON.stringify(snapshot.usage).slice(0, 300) : "(no usage)";
    line(`usage: ${usage}`);
    line(`summary: ${String(snapshot.run.summary ?? snapshot.summary ?? "(none)").slice(0, 2000)}`);
    break;
  }
  await new Promise((done) => setTimeout(done, every));
}

console.log(`log: ${out}`);
