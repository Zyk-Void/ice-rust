# Selected TUI goal — completion follow-up

These items landed on `main` in PR #44 (`44072f0`, 2026-10-21). This file records
what they cover and what was not verified. Delivery of the older stacked PRs
(#6-#24, #33-#43) was replaced by #44.

## Previously missing code now implemented

- **TUI-067:** tree navigation offers no summary, summary, and summary with custom
  instructions. Navigation is cancellable, excludes concurrent user bash, keeps
  queued prompts and the draft, and replays only the selected active branch.
  The harness journals navigation and recovers committed-but-unfinalized
  summaries without another provider request. CLI startup applies
  `branchSummary.reserveTokens`; an additive harness getter/setter avoids new
  required fields in public options. See `../guides/branch-summaries.md`.
- **TUI-058:** `/context` includes bounded category accounting, a proportional
  grid and largest conversation entries, with estimates clearly distinguished
  from provider-reported context. It follows the active conversation branch.
- **Session lifecycle:** new/resume/import/fork share an acknowledged drain of
  old display events before replacing the backing session/UI, plus coordinated
  admission against active user bash. Old events cannot subsequently repopulate
  a freshly reset transcript or footer.
- **TUI-043:** completion summaries use authoritative harness outcomes, including
  extension-started runs. Aborted/failed runs do not display success. Completion
  follows an ordered finalization marker after final broadcasts, without waiting
  for a later run to become idle or freezing that later run's streaming.
  Navigation interrupts retain their ownership even if their task starts late.

No plugin ABI, required public options, or persisted schema was changed. The
existing navigation operation intent/result machinery is reused. Crash-recovery
semantics and the additive harness reserve API are documented in the
branch-summary guide. An additional public `AgentEvent::RunSettled { run_id }`
broadcast marker orders completion after final context
writes and before subsequent runs; downstream exhaustive event matches must
account for the new variant. This is distinct from the raw agent loop's earlier
`AgentEnd` and is not a persisted session record. The default JSON/RPC emission
paths filter this local marker, preserving the existing emitted wire event set;
plugin translation also ignores it.

## Verification status

At the merged head (`1009a42` before squash): `cargo fmt --check`, a
`-D warnings` check of all targets, **2,195 Rust tests passed with none failed
and five ignored**, embedded-doc parity, and the four release-helper Python tests
all passed locally. Hosted CI on #44 (Linux and Windows check, test and
installer jobs) passed. Two OpenCodeReview passes over the whole change were
triaged; confirmed findings were fixed with regression tests.

Commands (with a disposable `ICERS_CODING_AGENT_DIR`, `ICERS_OFFLINE=1`, and
compiler and rustdoc warnings denied):

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
python3 scripts/test_prebuilt_release.py
sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs
```

Baseline commit: `e49cf6e56bc0a3aa4642d18569902919c0f0a66c`. The tested compiled
workspace file digest was unchanged before/after the run:
`20ea2415089390e9e8c55c609e292a1db9a9da521efb69fc2be44e73becdbb88`.
Evidence: `.validation/final/results.json` and `.validation/final/output.log`.
These task-local artifacts are not product files or committed CI evidence.

Focused suites passed on the integrated source, with no skipped regression:

| Suite | Passed | Coverage |
|---|---:|---|
| CLI `session_drain` | 18 | Replacement barriers, authoritative abort, ordered summaries, delayed interrupts, active-branch replay |
| CLI `tree_summary` | 17 | Chooser/custom instructions, cancellation, ownership, already-at-parent/root prompt replay and preserved drafts/images/queues |
| CLI `context_usage` | 10 | Bounded widths/allocation, provider usage, active branch, identity fences and read-only publication |
| Harness full suite | 395 | Includes 11 navigation/recovery/budget tests and 2 new settlement/abort tests |
| CLI/extensions `settled` | 5 | Local marker projection/filtering and unchanged plugin event translation |

The earlier red direct-extension ToolUse-abort test now passes unfiltered.
The read-only integration review found its three final race/replay findings
closed after the fixes; that review did not independently execute the tests.

Native preflight used an isolated config, a local synthetic provider and a real
Alacritty window. Nano 9.2 successfully owned the terminal through Ctrl+G, saved
a multiline Unicode draft, and returned with the caret at the final byte, no
extra trailing blank line, and no competing TUI repaint. This preflight is not
final-source certification. The GUI driver's background plain-Enter delivery was
unreliable, so its fixture config adds Ctrl+M as an additional submit/confirm
binding while retaining Enter. No real user configuration was changed.

## Remaining boundaries

- Real-terminal checks are not done: mouse scroll/partial-drag/clipboard, vim,
  `code --wait`, an editor opened during a streaming run, OSC 52 delivery over
  SSH, and terminal light/dark detection. Only a nano preflight ran, before the
  final source.
- Rust 1.78 was not run; the baseline already had dependency MSRV blockers and
  no dependency versions were changed.
- Resuming a session created in another directory keeps tools in the current
  directory; a note tells the user. Ctrl+C, Ctrl+D, Esc, Tab and Up/Down are
  fixed keys. `app.session.new/tree/fork/resume` ship unbound and are bindable
  in `keybindings.json`.
- Recovery reports a dangling deferred write when an append fails after a
  navigation move; this matches how failed run steps are already reported.
- `goal.md` §7.2 remains deferred and §7.3 excluded, as originally selected.
