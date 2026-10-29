# M5.4 local held-out preparation

This is preparation, not a JEV effectiveness result. M5.4 remains **BLOCKED**.
No model, JEV/TypeSafe, Cognee, OpenRouter or other remote service is used.
Arm D is not executed. The original revision is
`2b31a97e9c668f89e9b944dd5ded36f4af2f2d01`.

## Predeclared method (written before labeling)

The requested native local extractor does not exist in this revision.
`propose.rs` receives already-selected candidate text from the agent;
`dream.rs` selects ledger records and the runtime sends them to remote
`improve`. With explicit owner authorization, use two separate strata:

1. **Recorded proposals:** actual `cognee_propose_memory` tool calls found
   in the four allowed stores, replayed with their original source IDs.
2. **Synthetic proposal stream built from real user turns:** verbatim
   sentence spans from user turns, not actual icers proposals. Assistant
   and tool text never generates candidates.

The generator and matcher are ordinary deterministic Python, not an LLM.
The native replay is an ignored Rust test, requiring an explicit private
manifest path. It constructs no runtime/client/transport and writes no memory
queue. It calls the production `source_validation::validate_source` and
`candidates::admit` unchanged, with up to 256 prior/current turns. Recorded
proposals cannot see turns after their call; generated candidates see only
through their source turn. Pending means not admitted without a judge.
These are source/admission predicate decisions, not durable runtime writes
or a replay of live task ownership, repository state, queue capacity or
memory expiry. The current runtime's authoritative source gate is reused;
its runtime-only scope machinery is not fabricated from historical paths.

### Frozen generation rules

Seed: **20261028**. Session paths are sorted; sessions are processed by
start time, with content hash as tie-breaker. Selection is uniform
SHA-256 ranking of `(seed, session/pool, sentence)` without replacement.
At most five signal spans per session, then an equal number of other
spans when available; random-pool shortages stay in the counts.

Sentence regex: `[^\n.!?]+(?:[.!?](?=\s|$)|$)`; strip outer whitespace,
preserve the resulting exact substring, require 8–300 characters.
Long spans are not truncated to fit. This deliberately simple splitter
can split abbreviations and code/path punctuation.

Case-insensitive signal regexes:

```text
\b(?:always|never|don't|do not|prefer|please use|we decided|from now on|remember)\b
^(?:please\s+)?(?:use|keep|avoid|ensure|run|test|do|don't|stop|read|write|preserve|follow|must|only)\b
\b(?:must|must not|should not|required|constraint|instead of|rather than)\b
```

Kind hint: `preference` for `prefer`, `decision` for `we decided`, otherwise
`constraint` (including random-pool spans). Hints are not utility labels.
The native complete-statement check can reject sentence spans within a
larger source line; do not weaken the production validator to admit them.

### Frozen hindsight matcher

- Candidate reference time is session **end**. Evidence comes only from
  sessions whose start is strictly later, and from user turns strictly
  after that reference time and at most 14 days later, inclusive.
- Require exact equality of known scrubbed cwd/project. Unknown projects
  receive UNKNOWN. No repository inference or alias merging is attempted.
- Casefold; extract Unicode word tokens, length at least three; remove the
  frozen stop-word list recorded in the manifest.
- Require at least three distinct shared tokens and at least 70% of the
  candidate's remaining tokens in one later user sentence.
- Require two shared key terms (length at least six, or underscore/digit).
- Require equal presence/absence of `never|not|don't|avoid|mustn't|cannot`.
- A match yields USEFUL, with later session/turn evidence retained locally.
  Without a match, NOT_USEFUL requires a later same-project session starting
  at least 14 days after the reference time. Otherwise UNKNOWN.
- UNKNOWN remains in every combined/per-split/per-pool denominator. No
  threshold tuning on HELD_OUT; no semantic/manual/LLM labeling.

These are lexical hindsight proxies, not proof of correctness or causality.
They recognize restatements; implicit corrections and visible redo without
lexical overlap can be missed. Opposite meanings with the same vocabulary
can still match. Repeated boilerplate/injected prompt context can appear in
user envelopes and inflate restatement labels. The local audit is intended
for owner review, not retroactive threshold tuning.

### Normalization, privacy and freeze

Read only JSONL files beneath the four authorized roots; skip symlinks and
`.ice`, `.git`, `node_modules`, `target` directories. No credential files,
forbidden paths or live Cognee store are opened. Claude tool-result user
envelopes become tool turns; Codex `response_item` records are canonical,
not duplicate `event_msg` text. Icers and Least use native message entries;
stream frames and non-conversation events are not turns. Preserve file
order and report timestamp anomalies. Codex can reuse assistant message IDs
across content blocks; generated user-source IDs are checked for uniqueness.
Explicit null IDs receive deterministic turn-index fallbacks. Drop tool bodies over 4096 bytes to
a placeholder. Skip files with fewer than two user turns, report parse
failures rather than silently dropping them, deduplicate normalized
unscrubbed conversation content by SHA-256 before retaining a session.

Before persistence, scrub env assignments, credential-bearing URLs, Bearer
tokens, API-token prefixes, emails, `/home/mewtwo`, long hex and long
base64-like runs. Redaction counts describe normalization substitutions,
including metadata and proposals, not unique secrets. Tool bodies omitted
before scrubbing are not counted. Raw logs are never copied.

Private artifacts are exclusively under
`target/cognee-remaining-validation/heldout/`, directory mode 700 and file
mode 600. Preparation uses exclusive creation, refusing overwrite/symlinks.
`generation-manifest.json` freezes the rules and generator/matcher source
hash before reading the corpus; `protocol.json` is the native test input.
The final manifest hashes all derived data files except itself (its own
SHA-256 is reported separately), plus native extractor/admission and matcher
sources. Only the aggregate/hash manifest and this method report are public;
no source/session identifiers, cwd, candidate text or evidence spans are
committed. Harness code and synthetic-only tests are also committed.

DEV uses session starts before **2026-10-20 00:00 local**; HELD_OUT starts
then or later. Keep the cutoff unchanged even if known labels are under 300.
The blind audit draws 50 USEFUL and 50 NOT_USEFUL when possible, filling
shortages uniformly from remaining held-out candidates including UNKNOWN,
then shuffles by independent seeded hash rank. The static HTML escapes
session text, has no remote assets/network access, hides hindsight labels,
and downloads only candidate hashes and owner choices. Answers are not
submitted anywhere or saved automatically. Keep the downloaded JSON in the
private directory and set mode 600.

## Frozen results

| Source | JSONL files parsed | Eligible before dedup | Duplicates | Retained sessions | Fewer than two user turns |
|---|---:|---:|---:|---:|---:|
| Claude Code | 283 | 176 | 8 | 168 | 107 |
| Codex | 72 | 69 | 1 | 68 | 3 |
| icers | 67 | 17 | 0 | 17 | 50 |
| Least | 5 | 1 | 0 | 1 | 4 |
| **Total** | **427** | **263** | **9** | **254** | **164** |

All 427 files parsed; failed JSON lines, unreadable files, non-object lines,
unknown projects and missing session start/end timestamps: **0** each.
Timestamp regressions: Claude 164, icers 12, other sources 0. Oversized tool
bodies omitted: Claude 2,388; Codex 1,399; icers 129; Least 73 (3,989 total).
Date range: **2026-09-07 04:41:34.947 UTC – 2026-10-28 14:33:54.824 UTC**.
There are 132 DEV sessions and 122 HELD_OUT sessions.

**Recorded proposal stratum: 0 calls in every source and split.** The 1,150
candidates below are entirely the synthetic proposal stream. Pools balance
exactly: 575 signal and 575 random; no random-pool shortages. All candidate
source IDs uniquely identify a turn; every generated candidate is verbatim
within its scrubbed source. Reused Codex assistant block IDs do not affect
these user-source candidates.

| Split / pool | Total | USEFUL | NOT_USEFUL | UNKNOWN | Arm C admit | Arm C reject |
|---|---:|---:|---:|---:|---:|---:|
| DEV signal | 258 | 34 | 21 | 203 | 62 | 196 |
| DEV random | 258 | 40 | 21 | 197 | 40 | 218 |
| **DEV combined** | **516** | **74** | **42** | **400** | **102** | **414** |
| HELD_OUT signal | 317 | 57 | 0 | 260 | 69 | 248 |
| HELD_OUT random | 317 | 61 | 0 | 256 | 103 | 214 |
| **HELD_OUT combined** | **634** | **118** | **0** | **516** | **172** | **462** |
| **Overall** | **1,150** | **192** | **42** | **916** | **274** | **876** |

Native decision reasons: **274 `source_validated_local_accept`**, **876
`unsupported_source_span`**. No production admission code was changed.
The high rejection rate reflects the strict complete-source-statement gate,
not a calibrated usefulness filter. This proposal-stream mismatch is a
material limit for interpreting a future C-versus-D comparison.

UNKNOWN_TIME: 0 candidates. All UNKNOWN labels stay in denominators.
HELD_OUT has **118 known labels**, below 300. Its latest possible 14-day
negative window extends beyond the corpus end; zero held-out negatives is
right censoring, not evidence all candidates are useful. **The cutoff is
unchanged: 2026-10-20 00:00 +03:00 (1792443600000 ms).**

For a separately approved new freeze, propose **2026-10-01 00:00 local**:
it retains an earlier development interval and gives more held-out sessions
14 days of follow-up. This is a proposal only, not an applied change or a
promise of 300 known labels. No alternate-cutoff matcher run was performed.
Extending source history instead would reduce censoring without moving the
original split. No automatic resampling or continuation until success.

### Privacy accounting

| Redaction kind | Substitutions |
|---|---:|
| API-token prefixes | 40 |
| Bearer credentials | 78 |
| Emails | 156 |
| Env-style assignments | 3,571 |
| Home paths | 13,622 |
| Long base64-like runs | 16,934 |
| Long hex runs | 930 |
| URL credentials | 4 |
| **Total** | **35,335** |

Counts include repeated appearances; long-run rules also redact benign IDs.
The original extraction was archived locally before labeling after a null
source-ID parser correction; all six prelabel files are included in the
freeze hashes. Normalization was rerun against the allowed logs (which can
still grow); the final source inventory freezes exact input byte hashes.
No matcher, signal pattern or sampling threshold was changed based on labels.
Active session files were not excluded; the byte-hash inventory, not a claim
of an immutable upstream log store, defines this snapshot. Logs include
agent-shaped user envelopes and potentially duplicated/forked history;
whole-session hash deduplication does not remove shared conversation prefixes.
Kind hints are highly skewed: 1,145 constraints, 5 preferences, 0 decisions,
lessons or outcomes. Do not generalize this cohort to every proposal kind.
The validation target disappeared externally during setup and was recreated;
no source logs were modified. The initial focused compile found an unboxed
tool-result fixture; corrected before the successful corpus replay.

### Reproduction (fresh private output directory only)

```sh
python3 -I scripts/test_prepare_cognee_heldout.py
python3 -I scripts/prepare_cognee_heldout.py prepare
export CARGO_TARGET_DIR=target/cognee-remaining-validation CARGO_BUILD_JOBS=2
export RUSTFLAGS='-D warnings' RUSTDOCFLAGS='-D warnings'
export CARGO_NET_OFFLINE=true ICERS_OFFLINE=1
export ICERS_CODING_AGENT_DIR="$(mktemp -d)"
export ICERS_HELDOUT_MANIFEST="$PWD/target/cognee-remaining-validation/heldout/protocol.json"
cargo test -p ice-cli --lib cognee::heldout_tests::replay_heldout_native_admission --locked -- --ignored --exact --nocapture
python3 -I scripts/prepare_cognee_heldout.py freeze
sh scripts/verify_cognee_heldout.sh
```

Do not delete/overwrite this freeze to rerun it. Archive its private directory
first. A new read of actively growing logs can have a different input hash.
No command above contacts a provider or starts Cognee.

### Audit and manifest

Open the local-only file:
`target/cognee-remaining-validation/heldout/audit.html`.

The 100-candidate sample is **59 USEFUL, 0 NOT_USEFUL, 41 UNKNOWN**. With no
held-out negatives, the frozen rule first selected 50 positives and then
filled from all remaining candidates, yielding another 9 positives and 41
unknowns. The page itself does not expose these labels. Owner answers have
not been collected; no human validation or inter-rater agreement is claimed.

Private manifest:
`target/cognee-remaining-validation/heldout/manifest.json`.
SHA-256: `b6464500e79c8ccef6ac407133f50e24691808b736918e37f2938632e122ec15`.

The committed `evidence/heldout-manifest.json` is an aggregate/hash-only
projection: it retains counts, timestamps, seed, source/data hashes and the
private manifest hash; replaces the text-bearing protocol object with its
hash. All 16 derived data files (including the archived prelabel extraction)
are hashed; the final manifest excludes itself to avoid recursive hashing.
Git HEAD records the starting revision; source hashes identify the exact
uncommitted harness used in this freeze and must match the eventual commit.

### Verification

Full required gate logs and exit codes are retained locally under
`target/cognee-remaining-validation/heldout-gates/`. All final gates passed:

| Gate | Exit | Result |
|---|---:|---|
| Synthetic preparation tests | 0 | 19 passed |
| `cargo fmt --all -- --check` | 0 | Clean |
| `cargo check --workspace --all-targets --locked` | 0 | No warnings/errors |
| `cargo test --workspace --locked --no-fail-fast` | 0 | 3,443 parent-suite tests passed, 0 failed, 12 ignored |
| Embedded docs sync and diff | 0 | No generated-doc changes |
| `python3 -I scripts/test_prebuilt_release.py` | 0 | 4 passed |
| `git diff --check` | 0 | Clean |

The Rust log also contains four subprocess test summaries (3 CLI children
and 1 JSONL ownership child). Summing every printed summary gives 3,447
passes; the non-double-counted parent-suite total is **3,443**. The new
ignored corpus replay passed separately (1 passed, 1,545 filtered), covering
all 1,150 candidates. The new synthetic Rust test is included in the parent
workspace count. No ignored live test was enabled by the workspace gate.

All Rust commands used `CARGO_TARGET_DIR=target/cognee-remaining-validation`,
`CARGO_BUILD_JOBS=2`, `RUSTFLAGS=-D warnings`, `RUSTDOCFLAGS=-D warnings`,
`CARGO_NET_OFFLINE=true`, `ICERS_OFFLINE=1`, and a fresh temporary
`ICERS_CODING_AGENT_DIR`. No dependency fetch or service/model request was
needed. Hash verification found zero data/source mismatches and zero private
permission violations; label checks found zero temporal/project/role
violations. The 100 audit IDs are unique and all belong to HELD_OUT.
