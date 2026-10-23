#!/usr/bin/env node
// Recipe adapter for context-mode 1.0.169; no changes to the installed package.
import { readFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const [packageRoot, stateDir] = process.argv.slice(2);
if (!packageRoot || !stateDir) {
  throw new Error("Usage: node context-mode-hook.mjs PACKAGE_ROOT STATE_DIR");
}
mkdirSync(resolve(stateDir), { recursive: true, mode: 0o700 });
const input = JSON.parse(readFileSync(0, "utf8"));
// The core already aliases bash/read/grep. Its MCP leaf parser needs __.
if (input.tool_name?.startsWith("context-mode_")) {
  input.tool_name = `mcp__context-mode__${input.tool_name.slice("context-mode_".length)}`;
}
const child = spawnSync(process.execPath, [resolve(packageRoot, "hooks/pretooluse.mjs")], {
  input: JSON.stringify(input),
  encoding: "utf8",
  maxBuffer: 1024 * 1024,
  env: { ...process.env, CLAUDE_CONFIG_DIR: resolve(stateDir) },
});
if (child.error) throw child.error;
process.stderr.write(child.stderr ?? "");
// Translate guidance only; never rewrite updatedInput (executable arguments).
if (child.stdout?.trim()) {
  const output = JSON.parse(child.stdout);
  for (const object of [output, output.hookSpecificOutput]) {
    if (!object) continue;
    for (const key of ["reason", "permissionDecisionReason", "additionalContext"]) {
      if (typeof object[key] === "string") {
        object[key] = object[key]
          .replaceAll("mcp__plugin_context-mode_context-mode__", "context-mode_")
          .replaceAll("mcp__context-mode__", "context-mode_");
      }
    }
  }
  process.stdout.write(`${JSON.stringify(output)}\n`);
}
process.exit(child.status ?? 1);
