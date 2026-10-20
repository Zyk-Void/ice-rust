# Extensions audit checkpoint — ready for coordinator review

The assigned extension draft and proposal are complete for independent review. They are not published; proposal.complete remains false for coordinator acceptance. Original EXT-001 through EXT-034 are preserved, corrected and backed up in spec-extensions-before-review-2026-10-20.json. EXT-035 through EXT-057 finish tool metadata/rendering, UI controls and host actions.

## Artifacts and identities

- spec-extensions.json: 57 entries; SHA-256 546017278ccbc07430f1d8e7f0d5674d52c1d10ec7e9d808707505d9f7d6f6b4.
- proposal-extensions.json: SHA-256 9022be764eeebe23f8f66ebd7e22db2321b9a74db9cb420a11f357e3af4d9af4; eight original ledger files, 271 surfaces (251 candidate-backed, 20 manual), seven explained mechanical exclusions, all 258 original candidates accounted exactly once, eight reconciled first-pass claims.
- extensions-source-review.json: current actual raw read ranges and hashes; SHA-256 21c5aed671a39399290b6198aece6f5d49bfb6b04347f3f340a025bd4811b5da.
- extensions-test-identities.json: 15 inspected TS test files; 14 exactly match frozen Git HEAD and one modified input-event test is explicitly current-only. No tests ran.
- extensions-native-searches.json, extensions-defect-candidates.json and extensions-final-qa.json contain bounded search evidence, source-derived findings and review checks.

## Verification

TS HEAD 3ab05193147f5abb8eb8ee6c0178f6dbce8918b0; Rust HEAD ff9270f26e263ea9ce54349786bf2d93c4517476. All 65 inspected files match their recorded read hashes; all present frozen source-manifest hashes match. Fifty-two files are fully read, including every original extension ledger file and the complete native SDK/loader/registry/registration/translation/tool/actions/resource/provider-hook/prompt/UI mailbox implementations used for comparison. Thirteen supplemental cross-area files retain exact partial reads below.

Read-only inspect-spec passed 423 unique source references with no problems. Exact quoted schemas match source; all cited ranges are covered by actual reads. These are authoring/identity checks, not execution tests. All original IDs and all nine entry fields remain. The first-pass report ICE_PORTING.md was read at1–48,431–464,648–678 (SHA-256 d66e4d33ef0023d228110786f03aeb73c97e72df454b1e481f375b3ac264d7fc).

## Precise review needs

Independently review EXT-034 project-trust hook/default/admission behavior with SET-003; EXT-037/038 kind-only renderer dispatch; EXT-033 BeforeAgentStart and stale-dispatch payload cleanup plus ownership documentation; EXT-014 shortcut owner/modifier routing; EXT-020 silent reload signal outcome. Resolve proposal.dependency_requests and inherited AGENT links. Existing native panel/dialog/status/editor capabilities are classified explicitly instead of blanket missing claims. Test/source uncertainty and native-specific ownership design are preserved. Source findings were not reproduced by running plugins.

## Exact unread supplemental ranges

No original ledger or cited entry range remains unread. These uncited portions belong to broader caller/test/report contexts and are not claimed as fully reviewed:

- ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts: 1–879, 941–2769, 3821–4579, 4741–5569, 5659–8282
- ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts: 1–279, 606–2253
- rust:crates/ice-cli/src/interactive_tui.rs: 1–94, 256–269, 526–3398
- rust:crates/ice-cli/src/interactive_tui/sessions.rs: 1–424, 611–679
- ice:packages/coding-agent/src/core/agent-session.ts: 1–1079, 1311–1629, 1831–1919, 1961–2019, 2089–2854, 3136–3484, 3526–3999, 4031–4036
- ice:packages/coding-agent/test/assistant-message.test.ts: 1–119
- rust:crates/ice-cli/src/session.rs: 1–439, 540–1394, 1444–1839, 1941–4016
- rust:crates/ice-cli/src/config.rs: 1–469, 516–1761
- rust:crates/ice-harness/src/agent_harness/child.rs: 161–300
- rust:crates/ice-cli/src/args.rs: 1–469, 483–1121
- ice:packages/coding-agent/test/suite/rpc-contract.test.ts: 1–629, 705–1131
- ice:packages/coding-agent/src/core/resource-loader.ts: 1–659, 701–1019, 1066–1177
- rust:ICE_PORTING.md: 49–430, 465–647

## Next authorized action

Root reviews and publishes extensions, then tools, and owns shared Markdown/ledger/surface/progress changes and final relocation. This worker now proceeds to independent source-first QA of OBS-013, OBS-014, OBS-016 and OBS-018, saving qa-observability-independent.json, then the assigned full TUI investigation. No application source changes, tests, installs, network commands, commits, shared publication or real ~/.ice / ~/.icers access occurred.
