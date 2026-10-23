// Optional integration checks against the installed context-mode 1.0.169 package.
// CONTEXT_MODE_PACKAGE_ROOT=/absolute/package node --test context-mode-hook.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const packageRoot = process.env.CONTEXT_MODE_PACKAGE_ROOT;
const wrapper = fileURLToPath(new URL("context-mode-hook.mjs", import.meta.url));
for (const toolName of ["bash", "context-mode_ctx_execute"]) {
  test(`${toolName} receives context-mode routing`, { skip: !packageRoot }, () => {
    const root = mkdtempSync(join(tmpdir(), "icers-context-mode-test-"));
    try {
      const toolInput = toolName === "bash"
        ? { command: "curl -s https://example.com" }
        : { language: "shell", code: "curl -s https://example.com" };
      const result = spawnSync(process.execPath, [wrapper, packageRoot, root], {
        input: JSON.stringify({
          hook_event_name: "PreToolUse", session_id: "test", cwd: root,
          tool_name: toolName, tool_input: toolInput,
        }),
        encoding: "utf8",
      });
      assert.equal(result.status, 0, result.stderr);
      const output = JSON.parse(result.stdout).hookSpecificOutput;
      if (toolName === "bash") {
        assert.equal(output.permissionDecision, "deny");
        assert.match(output.permissionDecisionReason, /curl\/wget redirected/);
        assert.match(output.permissionDecisionReason, /context-mode_ctx_execute/);
        assert.doesNotMatch(output.permissionDecisionReason, /mcp__/);
      } else {
        // ctx_execute's curl runs in the sandbox, not the built-in bash redirect.
        assert.notEqual(output?.permissionDecision, "deny");
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}
