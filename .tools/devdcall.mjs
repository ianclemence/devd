#!/usr/bin/env node
// Call a devd capability and print what it answered.
//
//   node devdcall.mjs <capability> [--arg key=value ...] [--args '{json}']
//
// Everything after the capability is handed to devd unchanged, so the arguments are the ones the
// CLI documents: `--arg` for one value, `--args` for a whole object.
import { execFileSync } from "node:child_process";

const [id, ...rest] = process.argv.slice(2);
if (!id) {
  console.error("usage: devdcall.mjs <capability> [--arg key=value]... [--args '{json}']");
  process.exit(2);
}
const home = process.env.DEVD_HOME ?? "D:\\devd";
try {
  const out = execFileSync(`${home}\\devd.exe`, ["call", id, ...rest], {
    encoding: "utf8",
    env: { ...process.env, DEVD_HOME: home },
    maxBuffer: 64 * 1024 * 1024,
  });
  process.stdout.write(out);
} catch (error) {
  if (error.stdout) process.stdout.write(error.stdout);
  if (error.stderr) process.stderr.write(error.stderr);
  process.exit(error.status ?? 1);
}
