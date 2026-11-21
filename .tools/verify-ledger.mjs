#!/usr/bin/env node
// Check the ledger from the outside: start it, use it the way the README says, and stop it.
//
//   node verify-ledger.mjs
//
// Every assertion is about what a client sees or what the database holds afterwards — the two
// things a person can be wrong about independently.
import { spawn, execFileSync } from "node:child_process";
import { rmSync } from "node:fs";

const project = "D:\\devd\\www\\welcome\\ledger";
const port = 4390;
const db = "D:\\devd\\www\\devd\\.tools\\ledger-verify.db";
const base = `http://127.0.0.1:${port}`;
const KEY = "verify-key";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? " — " + detail : ""}`);
};

for (const suffix of ["", "-wal", "-shm"]) rmSync(db + suffix, { force: true });

const child = spawn(process.execPath, ["src/server.js"], {
  cwd: project,
  env: { ...process.env, PORT: String(port), DB: db, LEDGER_KEY: KEY },
  stdio: ["ignore", "pipe", "pipe"],
});
let out = "";
child.stdout.on("data", (c) => (out += c));
child.stderr.on("data", (c) => (out += c));

const call = async (path, { method = "GET", body, key = KEY } = {}) => {
  const answer = await fetch(`${base}${path}`, {
    method,
    headers: { "content-type": "application/json", ...(key ? { "x-ledger-key": key } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await answer.text();
  return { status: answer.status, body: text ? JSON.parse(text) : null };
};
const sql = (statement) => execFileSync(process.execPath, ["-e", `const {DatabaseSync}=require('node:sqlite');const db=new DatabaseSync(${JSON.stringify(db)});console.log(JSON.stringify(db.prepare(${JSON.stringify(statement)}).all()))`], { encoding: "utf8" }).trim();

let up = false;
for (let i = 0; i < 100; i += 1) {
  try {
    const answer = await fetch(`${base}/api/health`);
    if (answer.ok) {
      up = true;
      break;
    }
  } catch {
    /* not yet */
  }
  await new Promise((done) => setTimeout(done, 100));
}

try {
  check("the service starts and answers health without a key", up);

  check("a route without the key is 401", (await call("/api/accounts", { key: null })).status === 401);
  check("a route with a wrong key is 401", (await call("/api/accounts", { key: "nope" })).status === 401);

  const cash = await call("/api/accounts", { method: "POST", body: { name: "Cash", kind: "asset" } });
  const food = await call("/api/accounts", { method: "POST", body: { name: "Food", kind: "expense" } });
  const salary = await call("/api/accounts", { method: "POST", body: { name: "Salary", kind: "income" } });
  check("an account is created", cash.status === 201 && cash.body.name === "Cash", String(cash.status));
  check("a repeated name is a 409", (await call("/api/accounts", { method: "POST", body: { name: "Cash", kind: "asset" } })).status === 409);
  check("a bad kind is 400", (await call("/api/accounts", { method: "POST", body: { name: "Odd", kind: "magic" } })).status === 400);

  // Salary paid in: expense up, income credited (negative in a double-entry ledger).
  const pay = await call("/api/transactions", {
    method: "POST",
    body: { description: "Pay", posted_at: "2026-11-20T09:00:00Z", idempotency_key: "pay-1", entries: [{ account: "Cash", amount: 10000 }, { account: "Salary", amount: -10000 }] },
  });
  check("a balanced transaction is created", pay.status === 201 && pay.body.entries?.length === 2, String(pay.status));

  const lunch = await call("/api/transactions", {
    method: "POST",
    body: { description: "Lunch", posted_at: "2026-11-20T12:00:00Z", idempotency_key: "lunch-1", entries: [{ account: "Food", amount: 450 }, { account: "Cash", amount: -450 }] },
  });
  check("a second transaction is created", lunch.status === 201);

  const before = sql("SELECT (SELECT COUNT(*) FROM transactions) AS t, (SELECT COUNT(*) FROM entries) AS e");
  const bad = await call("/api/transactions", {
    method: "POST",
    body: { description: "Unbalanced", idempotency_key: "bad-1", entries: [{ account: "Cash", amount: -100 }, { account: "Food", amount: 50 }] },
  });
  const after = sql("SELECT (SELECT COUNT(*) FROM transactions) AS t, (SELECT COUNT(*) FROM entries) AS e");
  check("an unbalanced transaction is 400", bad.status === 400, String(bad.status));
  check("and writes nothing at all", before === after, `${before} -> ${after}`);

  const dup = await call("/api/transactions", {
    method: "POST",
    body: { description: "Pay", posted_at: "2026-11-20T09:00:00Z", idempotency_key: "pay-1", entries: [{ account: "Cash", amount: 10000 }, { account: "Salary", amount: -10000 }] },
  });
  check("the same key answers 200 with duplicate:true", dup.status === 200 && dup.body.duplicate === true, `${dup.status} ${JSON.stringify(dup.body.duplicate)}`);

  const [a, b] = await Promise.all([
    call("/api/transactions", { method: "POST", body: { description: "P1", posted_at: "2026-11-20T13:00:00Z", idempotency_key: "par-1", entries: [{ account: "Food", amount: 100 }, { account: "Cash", amount: -100 }] } }),
    call("/api/transactions", { method: "POST", body: { description: "P2", posted_at: "2026-11-20T13:00:00Z", idempotency_key: "par-2", entries: [{ account: "Food", amount: 200 }, { account: "Cash", amount: -200 }] } }),
  ]);
  check("two parallel posts both land", a.status === 201 && b.status === 201, `${a.status}/${b.status}`);

  const balances = await call("/api/balances");
  const byName = Object.fromEntries((balances.body.accounts ?? []).map((x) => [x.name, x.balance_cents]));
  const total = balances.body.total_cents ?? balances.body.total;
  check("balances add up", byName.Cash === 10000 - 450 - 100 - 200 && byName.Food === 450 + 100 + 200, JSON.stringify(byName));
  check("a balanced ledger totals zero", total === 0, String(total));
  check("balances are integers", Object.values(byName).every(Number.isInteger));

  const report = await call("/api/report?month=2026-11");
  check(
    "the report reads kinds, not signs",
    report.body.income_cents === 10000 && report.body.expense_cents === 450 + 100 + 200,
    JSON.stringify(report.body),
  );
  check("an empty month is zeros", (await call("/api/report?month=2025-01")).body.net_cents === 0);

  const archive = await call("/api/accounts/Cash/archive", { method: "POST" });
  check("an account archives", archive.status === 200 || archive.status === 204, String(archive.status));
  const archived = await call("/api/transactions", {
    method: "POST",
    body: { description: "After archive", idempotency_key: "arch-1", entries: [{ account: "Cash", amount: -100 }, { account: "Food", amount: 100 }] },
  });
  check("an archived account is refused in a new transaction", archived.status === 400, String(archived.status));

  const page = await fetch(`${base}/?key=${KEY}`);
  const html = await page.text();
  check("the page is served with the key", page.status === 200 && /text\/html/.test(page.headers.get("content-type") ?? ""), String(page.status));
  check("the page has a two-line form", (html.match(/<input[^>]+name="account"/gi) ?? []).length >= 2 || /entry-2|second line/i.test(html));
} finally {
  child.kill();
  await new Promise((done) => setTimeout(done, 300));
}

const listening = await new Promise((done) => {
  fetch(`${base}/api/health`).then(() => done(true)).catch(() => done(false));
});
check("nothing is left listening", !listening);

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
