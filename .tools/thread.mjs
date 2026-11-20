#!/usr/bin/env node
// One conversation, as lines: what each event was, who said it, and the words or the tool.
//
//   node thread.mjs <thread-id> [--events N]
//
// The daemon records turns as they happen; this prints the record, not a guess about it.
const base = process.env.DEVD_URL ?? "http://127.0.0.1:8787";

async function token() {
  if (process.env.DEVD_TOKEN) return process.env.DEVd_TOKEN;
  const page = await (await fetch(`${base}/`)).text();
  return page.match(/name="devd-token" content="([^"]+)"/)[1];
}

const id = process.argv[2];
if (!id) {
  console.error("usage: thread.mjs <thread-id> [--events N]");
  process.exit(2);
}
const answer = await fetch(`${base}/api/invoke`, {
  method: "POST",
  headers: { "content-type": "application/json", authorization: `Bearer ${await token()}` },
  body: JSON.stringify({ id: "chat.get", args: { thread: id } }),
});
const { outcome } = await answer.json();
const thread = outcome.data.thread;
console.log(`thread ${thread.id}`);
console.log(`  title   ${thread.title}`);
console.log(`  state   ${thread.state}   effort=${thread.effort ?? "default"}   model=${thread.model ?? "provider default"}`);
console.log(`  tokens  in=${thread.input_tokens} out=${thread.output_tokens} cost=${thread.cost_usd ?? "-"}`);
if (thread.error) console.log(`  error   ${thread.error}`);
console.log(`  running ${outcome.data.running}`);
for (const event of outcome.data.events) {
  const data = event.data ?? {};
  if (event.kind === "assistant") {
    const model = data.model ?? "?";
    for (const block of data.content ?? []) {
      if (block.type === "text") console.log(`  assistant[${model}] ${String(block.text).replace(/\s+/g, " ").slice(0, 160)}`);
      else if (block.type === "tool_use") console.log(`  tool_use  ${block.name} ${JSON.stringify(block.input).slice(0, 120)}`);
    }
  } else if (event.kind === "tool_result") {
    console.log(`  result    ${String(data.content ?? "").replace(/\s+/g, " ").slice(0, 140)}`);
  } else if (event.kind === "user") {
    console.log(`  user      ${String(data.text ?? "").replace(/\s+/g, " ").slice(0, 160)}`);
  } else {
    console.log(`  ${event.kind.padEnd(9)} ${JSON.stringify(data).slice(0, 140)}`);
  }
}
