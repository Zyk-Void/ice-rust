# Prompt: exhaustive ice → ice-rust gap audit (for GPT-6 Astra)

Recommended request settings: `model: gpt-6-astra`, `reasoning.effort: "xhigh"`
(set it on every request), Responses API with shell/file tools, working directory
`/home/mewtwo/Zks`.

---

## Goal

Produce a complete, source-referenced specification of every capability that the
TypeScript project **ice** (`/home/mewtwo/Zks/ice`) has and the Rust project
**ice-rust** (`/home/mewtwo/Zks/ice-rust`) lacks or implements differently. The
result is the porting specification for ice-rust. An engineer must be able to
implement each missing feature in Rust from your document alone, without
reopening the TypeScript source to work out how it behaves.

Document everything. Do not summarize, sample, or skip. Treat a missing element
as a failed audit. If something is large, split it into more entries. Do not
compress it into fewer.

## Context

- ice is a TypeScript coding-agent harness (a fork of pi-mono). Its packages are
  under `ice/packages/`: `ai`, `agent`, `coding-agent`, `tui`, `client`,
  `protocol`, `server`, `storage`, `evals`. It is about 185k lines of non-test TS.
  ice-only behavior is concentrated in `coding-agent/src/ice*.ts`,
  `coding-agent/src/subagents/**`, `coding-agent/src/observe/**`, the launcher
  `coding-agent/src/ice.ts`, and `coding-agent/examples/extensions/ice-blackhole/**`.
  `ice/ICE_CHANGELOG.md` and `ice/packages/coding-agent/docs/*.md` describe
  intended behavior.
- ice-rust is a Rust port of upstream pi (forked from `bigfish1913/pi-rust`,
  rebranded `rpi` → `ice`). Crates are under `ice-rust/crates/`: `ice-ai`,
  `ice-agent`, `ice-tools`, `ice-harness`, `ice-cli`, `ice-tui`,
  `ice-plugin-sdk`, `ice-extensions`, `ice-telemetry`. Extensions are native
  C-ABI plugins. ice-rust cannot load TypeScript extensions, so every TS
  extension ice relies on is a gap unless Rust reimplements it.
- A first-pass report exists at `ice-rust/ICE_PORTING.md`. Use it as a lead
  list only. Re-verify every claim in it against source, and expect it to be
  incomplete.

## Authority and autonomy

This is a read-only audit. You are authorized to read any file in both trees,
run read-only shell commands (`git`, `rg`, `find`, `wc`, `cargo metadata`,
`cargo check`, running `--help` on built binaries), and write files only under
`ice-rust/docs/ice-gap/`. Do not ask for permission for any of this.

Do not modify source in either repo. Do not commit, push, install packages, or
make network calls. Ask the user a question only if a finding depends on intent
that the code and docs cannot settle. Otherwise record it as an open question
in the report and keep going.

## Method

Work in this order, and keep going until the completion criteria below are met.

1. **Build a file ledger first.** List every non-test source file in
   `ice/packages/*/src/**`, `ice/packages/coding-agent/examples/extensions/ice-*/**`,
   and every doc in `ice/packages/coding-agent/docs/`. Use
   `git ls-files`, and exclude `*.test.ts`, `test/`, `dist/`, `node_modules/`
   and generated model data. Generated files still get one ledger row each, but
   only to say what they generate. Write the ledger to
   `docs/ice-gap/ledger.csv` with columns
   `path,lines,area,entry_ids,status,notes`. Every row must end with at least
   one `entry_id`, or with status `not-applicable` and a stated reason (for
   example, a Node-only shim with no Rust meaning).
2. **Diff every enumerable surface mechanically,** then confirm each hit by
   reading code. Substring matches are not evidence. Surfaces:
   - CLI flags and subcommands, including the flags `ice.ts` and extensions
     register via `registerFlag`
   - slash commands
   - keybindings
   - settings keys, including nested and namespaced `ice.*` keys, with their
     defaults
   - environment variables
   - model-facing tools and their parameter schemas and limits
   - RPC commands, responses, and events
   - extension API methods and events
   - session entry and custom message types (`customType` values)
   - files and directories ice reads or writes under `~/.ice` and `.ice/`
   - providers, wire APIs, and OAuth flows
   - TUI components and views
   - hook names
   - telemetry and observability events
3. **Read each ice subsystem fully.** Trace data flow from its entry point
   (launcher, CLI, extension factory, tool, command, event hook) to
   persistence and output. Read the relevant tests in `ice/packages/*/test/`
   to extract exact edge-case behavior, and cite them.
4. **Find the Rust counterpart** for every behavior. Search by behavior, not
   only by name: Rust may use different identifiers. Record whether it is
   present, partial, or missing, with Rust file and line references. For
   partial matches, list each behavioral difference.
5. **Delegate in parallel.** If you can parallelize work by delegating areas to
   subagents, do so. Use one area per agent: AI layer, agent loop, tools,
   session and compaction, settings and config, extensions host, RPC and
   remote, TUI, subagents, safe-verify and goals, observability, Cognee,
   Blackhole, providers and auth, packaging and CLI, misc. Give each subagent
   this whole prompt plus its area and its slice of the ledger. Merge the
   results yourself, then re-check that the ledger is fully covered.

## Required content for every gap entry

Each entry gets a stable ID (`AREA-NNN`) and all of these fields. Write `none`
when a field does not apply. Never omit a field.

1. **Title and status:** `missing`, `partial`, or `different-by-design`.
2. **ice source:** every relevant file path with line ranges, the exported
   symbols involved, and the tests that specify its behavior.
3. **What it does:** user-visible behavior in plain language.
4. **How it works:** entry points, control flow, and data flow step by step.
   Cover state and its transitions, persistence format and location, and
   concurrency, locking and ordering rules. Cover limits and constants with
   exact values, error handling and failure modes, and the security or trust
   checks involved.
5. **Interfaces:** exact tool parameter schemas, RPC message shapes, setting
   keys with types and defaults, env vars, CLI flags, slash commands,
   keybindings, events emitted and consumed, and file formats. Quote schemas
   verbatim where they are short.
6. **Dependencies:** other entries this one needs (by ID), and external
   services or binaries.
7. **ice-rust today:** Rust paths and lines, what exists, and the exact
   behavioral differences.
8. **Port notes:** the target crate or plugin, and the plugin host API
   additions it needs. Name Rust crates that cover it, if any. Note risks.
9. **Size and priority:** estimated Rust lines, and priority P0–P3 with a
   one-line reason.

## Deliverables

Write these under `ice-rust/docs/ice-gap/`:
- `README.md`: the index of all entries, as a table of ID, title, status,
  priority and size. Also include a dependency graph between entries, a
  suggested porting order, and open questions.
- One file per area (`ai.md`, `agent.md`, `tools.md`, `session.md`,
  `settings.md`, `extensions.md`, `rpc-remote.md`, `tui.md`, `subagents.md`,
  `safe-verify.md`, `observability.md`, `cognee.md`, `blackhole.md`,
  `providers-auth.md`, `cli-packaging.md`, `misc.md`), containing the full
  entries.
- `ledger.csv`, as described in Method step 1.
- `surfaces/`: one CSV per enumerable surface from Method step 2, with columns
  `name,ice_path:line,rust_path:line_or_blank,status,entry_id`.
- Also list things ice-rust has that ice lacks, in a short section of
  `README.md`, so they are not lost in the port.

## Completion criteria

You are done only when all of these hold:
- Every row of `ledger.csv` maps to an entry ID or is justified as `not-applicable`.
- Every item in every `surfaces/*.csv` file has a status and, unless it is
  `present`, an entry ID.
- Every entry has all 9 fields filled.
- Every claim in `ICE_PORTING.md` is either confirmed or corrected, with a list
  of corrections in `README.md`.
- A final self-check is recorded in `README.md`: ledger row count, mapped count,
  `not-applicable` count, surface item counts, and entry count per area.

Do not stop at a plan, a sample, or a partial pass. If context runs short,
write finished areas to disk first, then continue with the rest.

## Verification

Do not write tests. Verify claims by reading code, and cite file and line for
each claim. Where behavior is ambiguous, prefer the ice tests as the
specification and say so. Mark anything you could not confirm as `unverified`
in the entry, with the reason.

## Writing style

The reader is an engineer porting to Rust. Use plain, precise technical
language and exact identifiers, paths and values. Use tables for interfaces and
surfaces, and prose for mechanisms. No filler, no concluding summaries, no
marketing words. Put proper spaces between words and numbers.

## Final message

Report the counts from the self-check, the paths of the files written, the top
10 P0/P1 gaps by ID, and any open questions.
