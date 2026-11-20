#!/usr/bin/env node
// Talk to the running devd over its own HTTP API, with the session token the page carries.
//
//   node probe.mjs invoke <capability> '<json args>'   print the outcome as JSON
//   node probe.mjs raw <path>                          print the body of a GET
//
// The token is read from the page devd serves, so nothing secret has to be stored in a file.
import { readFileSync } from "node:fs";

const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";

async function token() {
  const cached = process.env.DEVD_TOKEN;
  if (cached) return cached;
  const page = await (await fetch(`${base}/`)).text();
  const found = page.match(/name="devd-token" content="([^"]+)"/);
  if (!found) throw new Error("the page carries no devd-token");
  return found[1];
}

async function invoke(id, args) {
  const answer = await fetch(`${base}/api/invoke`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${await token()}` },
    body: JSON.stringify({ id, args: args ?? {} }),
  });
  const body = await answer.json().catch(() => ({}));
  return { status: answer.status, body };
}

const [what, ...rest] = process.argv.slice(2);
if (what === "invoke") {
  const [id, json] = rest;
  if (!json) {
    const { status, body } = await invoke(id, {});
    process.stdout.write(JSON.stringify({ status, body }, null, 2) + "\n");
  } else {
    // A JSON argument on a command line is mangled by every shell on Windows, so it is read from a
    // file: `node probe.mjs invoke chat.send @args.json`.
    const text = (json.startsWith("@") ? readFileSync(json.slice(1), "utf8") : json).replace(
      /^\uFEFF/,
      "",
    );
    const { status, body } = await invoke(id, JSON.parse(text));
    process.stdout.write(JSON.stringify({ status, body }, null, 2) + "\n");
  }
} else if (what === "raw") {
  const answer = await fetch(`${base}${rest[0]}`);
  process.stdout.write((await answer.text()) + "\n");
} else {
  console.error("usage: probe.mjs invoke <capability> ['{json}'] | probe.mjs raw <path>");
  process.exit(2);
}
