#!/usr/bin/env node
// Verify quorum from the outside: three real nodes, a killed one, and a repaired one.
//
//   node verify-quorum.mjs
//
// Agreement is not testable from one process. This starts three, kills one, and asks the survivors
// what they believe.
import { spawn } from "node:child_process";
import { rmSync, mkdirSync } from "node:fs";

const project = "D:\\devd\\www\\welcome\\quorum";
const dataDir = "D:\\devd\\www\\devd\\.tools\\quorum-data";
const KEY = "verify-key";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? " — " + detail : ""}`);
};

rmSync(dataDir, { recursive: true, force: true });
mkdirSync(dataDir, { recursive: true });

const ports = [4521, 4522, 4523];
const ids = ["a", "b", "c"];
const peersOf = (self) => ports.filter((_, i) => ids[i] !== self).map((p) => `http://127.0.0.1:${p}`).join(",");
const nodes = new Map();

const start = async (id) => {
  const index = ids.indexOf(id);
  const child = spawn(process.execPath, ["src/node.js"], {
    cwd: project,
    env: {
      ...process.env,
      NODE_ID: id,
      PORT: String(ports[index]),
      PEERS: peersOf(id),
      CLUSTER_KEY: KEY,
      DB: `${dataDir}\\${id}.db`,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.on("data", () => {});
  child.stderr.on("data", () => {});
  nodes.set(id, child);
  for (let i = 0; i < 100; i += 1) {
    try {
      const answer = await fetch(`http://127.0.0.1:${ports[index]}/api/health`);
      if (answer.ok) return true;
    } catch {
      /* not yet */
    }
    await new Promise((done) => setTimeout(done, 100));
  }
  return false;
};
const kill = async (id) => {
  const child = nodes.get(id);
  if (!child) return;
  child.kill("SIGKILL");
  nodes.delete(id);
  await new Promise((done) => setTimeout(done, 300));
};
const call = async (id, path, { method = "GET", body } = {}) => {
  const port = ports[ids.indexOf(id)];
  const answer = await fetch(`http://127.0.0.1:${port}${path}`, {
    method,
    headers: { "content-type": "application/json", "x-cluster-key": KEY },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(8000),
  });
  const text = await answer.text();
  return { status: answer.status, body: text ? JSON.parse(text) : null };
};

try {
  const up = await Promise.all(ids.map(start));
  check("three nodes start", up.every(Boolean));

  const health = await call("a", "/api/health");
  check("health names the node, its peers and the quorum", health.body?.node === "a" && health.body?.quorum === 2, JSON.stringify(health.body));

  const written = await call("a", "/api/kv/colour", { method: "POST", body: { value: "green" } });
  check("a write through one node succeeds", written.status === 200, `${written.status} ${JSON.stringify(written.body)?.slice(0, 120)}`);
  check("it reports the acks it got", (written.body?.acks ?? 0) >= 2, JSON.stringify(written.body?.acks));

  const readB = await call("b", "/api/kv/colour");
  check("another node can read it", readB.status === 200 && readB.body?.value === "green", `${readB.status} ${JSON.stringify(readB.body)?.slice(0, 120)}`);

  // One node down: two of three is still a majority.
  await kill("c");
  const stillOk = await call("a", "/api/kv/colour", { method: "POST", body: { value: "blue" } });
  check("with one node dead a write still succeeds on two acks", stillOk.status === 200, `${stillOk.status} ${JSON.stringify(stillOk.body)?.slice(0, 120)}`);

  // Two down: no majority, so the write must be refused.
  await kill("b");
  const refused = await call("a", "/api/kv/colour", { method: "POST", body: { value: "red" } });
  check("with two dead the write is refused", refused.status === 503, `${refused.status} ${JSON.stringify(refused.body)?.slice(0, 160)}`);
  check("the refusal says why", typeof refused.body?.reason === "string" && refused.body.reason.length > 10, refused.body?.reason);

  // A node that is behind must be caught up by a read.
  const restartC = await start("c");
  check("the killed node starts again", restartC);
  const repaired = await call("c", "/api/kv/colour");
  check("its read returns the winning value", repaired.status === 200 && repaired.body?.value === "blue", `${repaired.status} ${JSON.stringify(repaired.body)?.slice(0, 140)}`);
  check("and the read says it repaired", repaired.body?.repaired === true, String(repaired.body?.repaired));

  const state = await call("a", "/api/state");
  check("state answers", state.status === 200, String(state.status));
  check("state reports the dead peer as unavailable", JSON.stringify(state.body ?? {}).includes("false") || JSON.stringify(state.body ?? {}).match(/down|unreachable|unavailable/i) !== null, JSON.stringify(state.body)?.slice(0, 200));
} finally {
  for (const id of [...nodes.keys()]) await kill(id);
}

const alive = await Promise.all(ports.map((p) => fetch(`http://127.0.0.1:${p}/api/health`).then(() => true).catch(() => false)));
check("nothing is left listening", alive.every((x) => !x), JSON.stringify(alive));

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
