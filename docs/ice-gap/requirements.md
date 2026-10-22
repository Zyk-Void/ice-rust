# Authoritative audit request

## Goal
Produce a complete, source-referenced specification of every capability that the TypeScript project ice (/home/mewtwo/Zks/ice) has and the Rust project ice-rust (/home/mewtwo/Zks/ice-rust) lacks or implements differently. The result is the porting specification for ice-rust. An engineer must be able to implement each missing feature in Rust from your document alone, without reopening the TypeScript source to work out how it behaves.

Document everything. Do not summarize, sample, or skip. Treat a missing element as a failed audit. If something is large, split it into more entries. Do not compress it into fewer.

## Context
ice is a TypeScript coding-agent harness (a fork of pi-mono). Its packages are under ice/packages/: ai, agent, coding-agent, tui, client, protocol, server, storage, evals. It is about 185k lines of non-test TS. ice-only behavior is concentrated in coding-agent/src/ice*.ts, coding-agent/src/subagents/**, coding-agent/src/observe/**, the launcher coding-agent/src/ice.ts, and coding-agent/examples/extensions/ice-blackhole/**. ice/ICE_CHANGELOG.md and ice/packages/coding-agent/docs/*.md describe intended behavior.

ice-rust is a Rust port of upstream pi, forked from bigfish1913/pi-rust at v0.6.1 (0fb2d893206981527b14e86d68e3ddbdf2c8189a) and mechanically rebranded from rpi to ice using scripts/ice-rebrand.sh. Zyk-Void/ice-rust is the downstream origin, not the upstream. Crates are under ice-rust/crates/: ice-ai, ice-agent, ice-tools, ice-harness, ice-cli, ice-tui, ice-plugin-sdk, ice-extensions, ice-telemetry. Extensions are native C-ABI plugins. ice-rust cannot load TypeScript extensions, so every TS extension ice relies on is a gap unless Rust reimplements it.

A first-pass report exists at ice-rust/ICE_PORTING.md. Use it as a lead list only. Re-verify every claim in it against source, and expect it to be incomplete. Repository-origin statements in that report or in this context must also be verified against local source and git metadata, not assumed.

## Authority and autonomy
This is a read-only audit. You are authorized to read any file in both trees, run read-only shell commands (git, rg, find, wc, cargo metadata, cargo check, running --help on built binaries), and write files only under ice-rust/docs/ice-gap/. Do not ask for permission for any of this.

Do not modify source in either repo. Do not commit, push, install packages, or make network calls. Ask the user a question only if a finding depends on intent that the code and docs cannot settle. Otherwise record it as an open question in the report and keep going.

## Method
Work in this order, and keep going until the completion criteria below are met.

1. Build a file ledger first. List every non-test source file in ice/packages/*/src/**, ice/packages/coding-agent/examples/extensions/ice-*/**, and every doc in ice/packages/coding-agent/docs/. Use git ls-files, and exclude *.test.ts, test/, dist/, node_modules/ and generated model data. Generated files still get one ledger row each, but only to say what they generate. Write the ledger to docs/ice-gap/ledger.csv with columns path,lines,area,entry_ids,status,notes. Every row must end with at least one entry_id, or with status not-applicable and a stated reason (for example, a Node-only shim with no Rust meaning).
2. Diff every enumerable surface mechanically, then confirm each hit by reading code. Substring matches are not evidence. Surfaces: CLI flags and subcommands, including flags ice.ts and extensions register via registerFlag; slash commands; keybindings; settings keys, including nested and namespaced ice.* keys with defaults; environment variables; model-facing tools and parameter schemas and limits; RPC commands, responses and events; extension API methods and events; session entry and custom message types (customType values); files and directories ice reads or writes under ~/.ice and .ice/; providers, wire APIs and OAuth flows; TUI components and views; hook names; telemetry and observability events.
3. Read each ice subsystem fully. Trace data flow from its entry point (launcher, CLI, extension factory, tool, command, event hook) to persistence and output. Read relevant tests in ice/packages/*/test/ to extract exact edge-case behavior and cite them.
4. Find the Rust counterpart for every behavior. Search by behavior, not only by name. Record present, partial or missing with Rust file and line references. For partial matches list each behavioral difference.
5. Delegate in parallel if available, one area per agent: AI layer; agent loop; tools; session and compaction; settings and config; extensions host; RPC and remote; TUI; subagents; safe-verify and goals; observability; Cognee; Blackhole; providers and auth; packaging and CLI; misc. Give each subagent this whole prompt plus its area and ledger slice. Merge and re-check coverage centrally.

## Every gap entry: all nine fields, never omit one; write none when inapplicable
1. Title and status: missing, partial, or different-by-design; stable AREA-NNN ID.
2. ice source: every relevant file path and line range, exported symbols and behavior-specifying tests.
3. What it does: user-visible behavior in plain language.
4. How it works: entry points, control flow, data flow, state/transitions, persistence format/location, concurrency/locking/ordering, exact limits/constants, errors/failures and security/trust checks.
5. Interfaces: exact tool parameter schemas, RPC shapes, setting keys/types/defaults, environment variables, CLI flags, slash commands, keybindings, emitted/consumed events and file formats. Quote short schemas verbatim.
6. Dependencies: other entry IDs and external services/binaries.
7. ice-rust today: Rust paths and lines, existing behavior and exact differences.
8. Port notes: target crate/plugin, required host API additions, applicable Rust crates and risks.
9. Size and priority: estimated Rust lines, P0-P3 and one-line reason.

## Deliverables under ice-rust/docs/ice-gap/
README.md: all-entry table (ID,title,status,priority,size), dependency graph, suggested order, open questions; also a short section of Rust-only capabilities to preserve.
Area files: ai.md, agent.md, tools.md, session.md, settings.md, extensions.md, rpc-remote.md, tui.md, subagents.md, safe-verify.md, observability.md, cognee.md, blackhole.md, providers-auth.md, cli-packaging.md, misc.md.
ledger.csv with the exact six columns above.
surfaces/: one CSV per enumerable surface with exact columns name,ice_path:line,rust_path:line_or_blank,status,entry_id.

## Completion criteria
Every ledger row maps to an entry ID or justified not-applicable.
Every surface item has a status and, unless present, an entry ID.
Every entry has all nine fields filled.
Every claim in ICE_PORTING.md is confirmed or corrected; README lists corrections.
README final self-check states ledger count, mapped count, not-applicable count, surface counts, entry count per area.
Do not stop at a plan, a sample, or a partial pass. Persist finished areas before context pressure and continue.

## Verification and writing
Do not write tests. Verify claims by reading code, cite file and line for each claim. Prefer tests when source behavior is ambiguous and say so. Explicitly mark unconfirmed behavior unverified with the reason. Write for a Rust engineer in precise technical language, exact identifiers/values, interface tables and mechanism prose. No filler, concluding summaries or marketing text. Use proper spaces between words and numbers.

## Final message
Report self-check counts, written paths, top 10 P0/P1 gap IDs and open questions.

## User override: no agents
The user subsequently instructed: "do not use any agents please". This overrides Method step 5. Continue directly without launching, delegating to, or resuming any agent. An earlier agent_plan call was dry-run only; no agent was launched. The preparation script was not used to delegate work.
