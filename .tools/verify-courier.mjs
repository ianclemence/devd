#!/usr/bin/env node
// Verify courier from the outside: a real service, real HTTP, a real restart on the same file.
//
//   node verify-courier.mjs
//
// The claims that matter are about time and death: no job handed out twice, a lease that expires,
// and a job that outlives the process that was working on it.
import { spawn } from "node:child_process";
import { rmSync } from "node:fs";

const project = "D:\\devd\\www\\welcome\\courier";
const db = "D:\\devd\\www\\devd\\.tools\\courier-verify.db";
const KEY = "verify-key";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? " — " + detail : ""}`);
};

for (const suffix of ["", "-wal", "-shm"]) rmSync(db + suffix, { force: true });

let port = 4410;
const base = () => `http://127.0.0.1:${port}`;
let child = null;

const start = async () => {
  child = spawn(process.execPath, ["src/server.js"], {
    cwd: project,
    env: { ...process.env, PORT: String(port), DB: db, COURIER_KEY: KEY },
    stdio: ["ignore", "pipe", "pipe"],
  });
  for (let i = 0; i < 100; i += 1) {
    try {
      const answer = await fetch(`${base()}/api/health`);
      if (answer.ok) return true;
    } catch {
      /* not yet */
    }
    await new Promise((done) => setTimeout(done, 100));
  }
  return false;
};
const stop = async () => {
  if (!child) return;
  child.kill("SIGKILL");
  child = null;
  await new Promise((done) => setTimeout(done, 300));
};
const call = async (path, { method = "GET", body, key = KEY } = {}) => {
  const answer = await fetch(`${base()}${path}`, {
    method,
    headers: { "content-type": "application/json", ...(key ? { "x-courier-key": key } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await answer.text();
  return { status: answer.status, body: text ? JSON.parse(text) : null };
};

try {
  check("the service starts", await start());

  check("a route without the key is 401", (await call("/api/jobs", { key: null })).status === 401);
  check("health is open", (await call("/api/health", { key: null })).status === 200);

  const enqueued = [];
  for (let i = 0; i < 5; i += 1) {
    const answer = await call("/api/jobs", { method: "POST", body: { queue: "q", payload: { i } } });
    enqueued.push(answer.body);
  }
  check("five jobs enqueued", enqueued.every((j) => j && j.state === "queued"));

  // Ten workers, dispatched together, must win five jobs and never the same one twice.
  const claims = await Promise.all(
    Array.from({ length: 10 }, (_, i) =>
      call("/api/jobs/claim", { method: "POST", body: { worker: `w${i}`, queue: "q", lease_seconds: 5 } }),
    ),
  );
  const won = claims.filter((c) => c.status === 200).map((c) => c.body);
  const ids = won.map((j) => j.id);
  check("five claims won, five got 204", won.length === 5 && claims.filter((c) => c.status === 204).length === 5, `${won.length} won`);
  check("no job was handed to two workers", new Set(ids).size === ids.length, ids.join(","));

  const bad = await call(`/api/jobs/${ids[0]}/ack`, { method: "POST", body: { worker: "not-the-holder" } });
  check("acking someone else's lease is 409", bad.status === 409, String(bad.status));

  // A worker that takes a lease and dies: the queue must take the job back when the lease expires.
  const abandoned = ids[0];
  await new Promise((done) => setTimeout(done, 5600));
  const swept = await call("/api/sweep", { method: "POST" });
  check("a sweep returns the expired lease", swept.status === 200 && swept.body.swept >= 1, JSON.stringify(swept.body));
  const reclaimed = await call("/api/jobs/claim", { method: "POST", body: { worker: "rescue", queue: "q", lease_seconds: 30 } });
  check("the abandoned job is claimable again", reclaimed.status === 200, String(reclaimed.status));
  const retried = reclaimed.body;
  check("its attempts moved on", retried && retried.attempts >= 2, JSON.stringify(retried?.attempts));

  // Durability: enqueue, claim, kill the process, start again on the same file.
  const dying = await call("/api/jobs", { method: "POST", body: { queue: "durable", payload: { x: 1 } } });
  const held = await call("/api/jobs/claim", { method: "POST", body: { worker: "doomed", queue: "durable", lease_seconds: 2 } });
  check("a job was claimed before the kill", held.status === 200 && held.body.id === dying.body.id);
  await stop();
  check("the service is gone", await new Promise((done) => fetch(`${base()}/api/health`).then(() => done(false)).catch(() => done(true))));

  port += 1;
  check("it starts again on the same file", await start());
  await new Promise((done) => setTimeout(done, 2200));
  const sweptAgain = await call("/api/sweep", { method: "POST" });
  const back = await call("/api/jobs/claim", { method: "POST", body: { worker: "heir", queue: "durable", lease_seconds: 30 } });
  check("the job survives the dead process", sweptAgain.status === 200 && back.status === 200 && back.body.id === dying.body.id, String(back.status));

  const nack = await call(`/api/jobs/${back.body.id}/nack`, { method: "POST", body: { worker: "heir", retry: false, error: "no handler" } });
  check("nack without retry sends it to dead", nack.status === 200 && nack.body.state === "dead", nack.body?.state);

  const page = await fetch(`${base()}/?key=${KEY}`);
  const html = await page.text();
  check("the page is served", page.status === 200 && /text\/html/.test(page.headers.get("content-type") ?? ""), String(page.status));
  check("the page shows the service's own state", /queued|dead|leased/i.test(html));
} finally {
  await stop();
}

const alive = await new Promise((done) => fetch(`${base()}/api/health`).then(() => done(true)).catch(() => done(false)));
check("nothing is left listening", !alive);

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
