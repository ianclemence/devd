#!/usr/bin/env node
// Ask the configured DeepSeek model one question, with thinking off, without going through devd.
//
// This is the wire probe: it proves what the provider accepts before devd is blamed for what it
// sends. The key is read from devd's own owner-only file and is never printed.
import { readFileSync } from "node:fs";

const key = readFileSync("D:\\devd\\secrets\\providers\\deepseek.key", "utf8").trim();
const model = process.argv[2] ?? "deepseek-flash";
const mode = process.argv[3] ?? "off";

const body = {
  model,
  messages: [
    { role: "system", content: "Answer in one short sentence." },
    { role: "user", content: "Name one benefit of keeping counters in memory." },
  ],
  max_tokens: 64,
  stream: false,
};
if (mode === "off") body.thinking = { type: "disabled" };

const started = Date.now();
const answer = await fetch("https://api.deepseek.com/v1/chat/completions", {
  method: "POST",
  headers: { "content-type": "application/json", authorization: `Bearer ${key}` },
  body: JSON.stringify(body),
});
const took = Date.now() - started;
const text = await answer.text();
let parsed;
try {
  parsed = JSON.parse(text);
} catch {
  parsed = null;
}
console.log(`model=${model} thinking=${mode} status=${answer.status} ${took}ms`);
if (!answer.ok) {
  console.log(text.slice(0, 600));
  process.exit(1);
}
const choice = parsed.choices?.[0];
console.log("finish_reason:", choice?.finish_reason);
console.log("content:", JSON.stringify(choice?.message?.content));
console.log("reasoning field:", JSON.stringify(choice?.message?.reasoning_content ?? null).slice(0, 200));
console.log("usage:", JSON.stringify(parsed.usage));
