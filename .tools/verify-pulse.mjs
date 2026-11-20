#!/usr/bin/env node
// Check the pulse service from the outside: start it, use it, read what it said, stop it.
//
//   node verify-pulse.mjs
//
// Every assertion is about what a client sees over HTTP or what the service printed, so this is
// evidence about the product rather than about its functions.
import { spawn } from "node:child_process";

const project = "D:\\devd\\www\\welcome\\pulse";
const port = 4301;
const base = `http://127.0.0.1:${port}`;
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? " — " + detail : ""}`);
};

const child = spawn(process.execPath, ["src/server.js"], {
  cwd: project,
  env: { ...process.env, PORT: String(port) },
  stdio: ["ignore", "pipe", "pipe"],
});
let stdout = "";
child.stdout.on("data", (chunk) => (stdout += chunk));
child.stderr.on("data", (chunk) => (stdout += chunk));

async function waitForListening() {
  for (let i = 0; i < 100; i += 1) {
    try {
      const answer = await fetch(`${base}/api/health`);
      if (answer.ok) return true;
    } catch {
      /* not up yet */
    }
    await new Promise((done) => setTimeout(done, 100));
  }
  return false;
}

try {
  check("the service starts and answers /api/health", await waitForListening());

  const first = await (await fetch(`${base}/api/metrics`)).json();
  check("at is an ISO timestamp", !Number.isNaN(Date.parse(first.at)), first.at);
  check("uptime_s is a number", typeof first.uptime_s === "number", String(first.uptime_s));
  check(
    "the reading request counts itself",
    first.counters?.requests >= 1,
    `requests=${first.counters?.requests}`,
  );
  check(
    "checks are node, memory, workspace in that order",
    (first.checks ?? []).map((c) => c.name).join(",") === "node,memory,workspace",
    (first.checks ?? []).map((c) => c.name).join(","),
  );
  const [node, memory, workspace] = first.checks ?? [];
  check("the node check reports process.version", node?.detail === process.version, node?.detail);
  check("the memory check states the ceiling", /^\d+ MB of 512 MB$/.test(memory?.detail ?? ""), memory?.detail);
  check("the workspace check actually wrote", workspace?.detail === "writable", workspace?.detail);
  check(
    "every check has a known status",
    (first.checks ?? []).every((c) => ["ok", "bad", "unknown"].includes(c.status)),
    JSON.stringify((first.checks ?? []).map((c) => c.status)),
  );

  const missing = await fetch(`${base}/nope`);
  check("an unknown path is 404", missing.status === 404, String(missing.status));

  const second = await (await fetch(`${base}/api/metrics`)).json();
  check(
    "a 404 raises requests and errors",
    second.counters.requests >= first.counters.requests + 2 && second.counters.errors === 1,
    `requests ${first.counters.requests}->${second.counters.requests}, errors=${second.counters.errors}`,
  );
  check(
    "errors do not reset on the next read",
    (await (await fetch(`${base}/api/metrics`)).json()).counters.errors === 1,
  );

  const page = await fetch(`${base}/`);
  const html = await page.text();
  check("the page is served", page.status === 200 && /text\/html/.test(page.headers.get("content-type") ?? ""));
  check(
    "the page carries the ids the board paints into",
    ["id=\"board\"", "id=\"status\"", "id=\"last\"", "Read now"].every((mark) => html.includes(mark)),
  );
  const module = await fetch(`${base}/render.js`);
  check("the board's module is served as JavaScript", /javascript/.test(module.headers.get("content-type") ?? ""));

  await new Promise((done) => setTimeout(done, 300));
  const lines = stdout.split(/\r?\n/).filter((line) => /^(GET|POST) \S+ \d{3} \d+ms$/.test(line));
  check(
    "one log line per request",
    lines.length >= 6,
    `${lines.length} lines, e.g. ${lines[0] ?? "-"}; ${lines[1] ?? "-"}`,
  );
} finally {
  child.kill();
}

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
