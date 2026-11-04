# Harness improvement execution log

Status: in progress. Branch: `fix/harness-audit-hardening`.
Baseline: `2fcc1618c0e81aec3e465c9fb24669a0d5655d84`.

Tracks [the implementation plan](harness-improvement-plan-2026-11-03.md).
No production data/configuration changes, pushes, PRs, or releases are authorized
by this execution. Tests use synthetic data and isolated agent directories.

## H17/H18 evaluation and follow-up fixes (2026-11-04)

**H17 (compaction semantics).** The owner picked 10 sessions (30 compactions).
A second model (gpt-6.1-sol) labelled each with a pre-compaction ledger; all
85 evidence quotes were verified verbatim against the sessions. First-pass
result: 27 lost something, 3 kept; dominant class "latest progress replaced by
an earlier snapshot" (17 cards); 2 critical (stale session/port directions);
every summary above 12k characters lost something. Labels are provisional
until the owner spot-checks them in the private review page. A comparison with
the TypeScript reference found the prompts identical, so the causes are
inherited. Code review found a structural bug: in one long turn every
compaction is a split turn with no prior history and the previous summary was
dropped ("No prior history."). PR #103 fixes it (test red before, green after)
and changes the prompts (older checkpoint first, reconcile to latest status,
Current State section, verbatim instructions and acceptance criteria). Live
summary eval on cx/gpt-6.1-terra: long-turn case 0/2 old, 2/2 new; four
shorter synthetic classes 12/12 on both (they do not separate the wordings).
File lists are only 8% of summary text and were left uncapped.

**H18 (task comparison).** Old (`main`) vs new build, 6 tasks x 3 reps on
cx/gpt-6.1-terra, isolated agent dirs outside any checkout. A first run was
invalid (run dirs inside the repo made icers load the repo's AGENTS.md; the
repo was not modified) and is archived. Valid run: fix-test, refactor, log
triage, resume after SIGKILL and denied write were equivalent (old 15/15, new
14/15, timing within noise at n=3). Compaction under a 48k window: old 3/3,
new 0/3. Cause: PR #96's request check refused a request its estimate put at
71k tokens; the provider measured 49k and accepted it on the old build. PR #96
commit 53b01ecc sends with a 4,096-token output floor instead of refusing;
recheck 2/3 complete and all compacted (the partial lost codewords across
compaction, an H17 class). Remaining gap: one turn of tool output can exceed
the window and overflow recovery cannot reduce the latest turn; a design is in
`target/harness-hardening-checks/research/h14-latest-turn-overflow-design.md`.

**Usage research (read-only mining of local sessions and telemetry).** Beyond
known issues: 656 re-reads of unchanged files (context cost), 84 still-running
`bash_output` polls (54 min blocked), 125 failed bash calls (6.4%), 11 edit
misses. Five "Unsupported content type" failures all fell in one hour on
2026-10-22 and have not recurred (historical Codex content-type incident).

## Telemetry-driven follow-up (2026-11-04, second agent)

The first agent stopped when its compaction summary hit the same Codex quota
429 described below. This pass took gaps observed in the read-only local
observability database (280 runs, 7.5k activity items), not new hypotheses.

- [x] H04: quota/usage-limit 429s with an hours-long reset were retried 10 times.
- [x] H02: unlabelled provider-shaped keys escaped redaction on legacy reads.
- [x] H08/H09: SIGTERM/SIGHUP left runs `running` and skipped all cleanup.
- [x] Pre-existing observer entry-ordering race exposed by the full gate.
- [x] Kept Cognee recall and improve work off the foreground path.
- [x] Split into review branches; verified each and the combined tree.
- [x] Final gates after these changes (see end of this section).

**H04 evidence.** The hardening session itself retried
`[429]: The usage limit has been reached (reset after 2h 14s)` and
`All codex accounts reached configured quota threshold (reset after 2h …)` ten
times (2 s → 60 s backoff, about six minutes) before failing. None of the six
distinct real messages matched the billing pattern. `is_permanent_account_error`
(shared by assistant retry, the harness agent loop/settle, compaction and the
Anthropic transport) now treats usage-limit/quota-threshold errors as
non-retryable when they state no reset or a reset beyond the 60 s default
backoff cap. A stated reset within the cap still gets bounded retries;
unrelated "reset after" text (for example a connection reset) is unaffected.
`quota-reset-before.log`: 7 passed, 2 failed (the two new tests), exit 101.
`quota-reset-after.log`: ice-ai 145 passed, 0 failed, exit 0.

**H02 evidence.** Legacy telemetry holds 3 `sk-` tokens (lengths 35 and 67)
and 13 URL-userinfo credentials; values were never printed. URL userinfo was
already masked on read; bare tokens without a `key=` label were not.
`redact_credential_text` now masks `sk-`, `ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`,
`github_pat_`, `AKIA`, `xox?-`, `AIza` and `glpat-` shapes that contain a digit
(so hyphenated words such as `sk-learn-compatible-…` survive). The same regex
applied to the legacy database masks all 3 stored tokens and matches nothing
else. Synthetic tokens are assembled at runtime in the test. Stored bytes are
unchanged; scrubbing them needs owner approval and a backup.
`bare-token-before.log`: 1 failed, exit 101; `bare-token-after.log`: observe
suite 91 passed, 0 failed, 1 ignored, exit 0.

**H08/H09 root cause (signals).** Only 13 of 280 recorded runs are
`finished`; 156 of the 267 `running` rows have at most two events. A real
process probe (`exit-probe-before.log`, isolated agent dir, unroutable synthetic
provider) showed RPC `stop` and stdin EOF finish correctly, but SIGTERM and
SIGHUP killed the process instantly: no SessionShutdown, Cognee shutdown,
durable observe finish or background-shell disposal. Fix:

- `modes::termination_signal()` resolves on SIGTERM/SIGHUP (Unix) or console
  close/shutdown (Windows). In RPC mode it ends the loop exactly like `stop`
  (abort and join the active run, then the existing teardown). The interactive
  TUI aborts the active run and queues `Exit`. A second signal force-exits
  (status 143) if graceful shutdown hangs. SIGINT behavior is unchanged.
- `bin/icers.rs` builds the runtime explicitly and uses a 500 ms
  `shutdown_timeout` after `run` returns. Without it, Tokio's pending blocking
  stdin read kept a signalled RPC process alive indefinitely after cleanup
  (`exit-probe-after.log` before this change: finished rows, but the process
  still alive after 10 s). Plain `stop`/EOF were never affected (53 ms exit).

`rpc-signal-before.log`: new loop regression timed out (1 failed, exit 101);
`rpc-signal-after.log`: 24 RPC tests passed, exit 0. Real-process results:
RPC SIGTERM exits in 510 ms with `finished` (installed pre-branch binary:
killed, `running`). TUI under a pty (`tui-signal-probe-3.log`, PID resolved via
`/proc/<pid>/exe` so no live icers process can be signalled): installed binary
4/4 SIGHUP/SIGTERM left `running`; fixed build 4/4 `finished`, exiting in
1.2–1.4 s. Print/JSON one-shot modes are not changed. SIGKILL, ACP's own
3 s kill fallback, and Windows behavior are not covered by these probes; the
Windows branch is type-checked only (below). Historical `running` rows are not
rewritten. This removes the dominant phantom-run cause for signalled exits; it
does not add process-instance identity, heartbeats or lease-based liveness
(H08 remains open).

**Observer entry ordering.** The first full gate failed
`observe::tests::a_run_is_recorded_with_prompts_custom_entries_and_finish`
(3,622 passed, 1 failed, 14 ignored; `signal-gates-workspace.log`); it passed
8/8 in isolation (`order-rerun-*.log`). Root cause predates this branch (same
logic at `HEAD`): a lagging forwarder's boundary read projected every new
session entry, so the next prompt and a custom entry appeared before the
earlier answer. Boundary reads now stop before an assistant entry whose
`message_end` has not been forwarded; `AgentEnd` still flushes, so an assistant
entry without an event can delay projection by one run at most, never block it.
`entry-order-before.log`: deterministic unit regression failed, exit 101;
`entry-order-after.log`: observe suite 92 passed, 0 failed, 1 ignored, exit 0.

**Cognee foreground and background cost (owner request: memory work must not
block the harness).** Correction to a first reading: only the Event policy's
explicit-history path records `foregroundWaitMs` (3 waits, about 4.4 s). The
default Legacy policy awaited recall on every prompt's first provider request,
up to `recallBudgetMs` (10 s); recorded recalls took about 4 s at the median.
The owner's live config currently has Cognee disabled, so this applies once it
is re-enabled. Legacy recall now waits at most 300 ms; a slower recall keeps
running as tracked background work and its memory is delivered once to a later
request of the same run, or dropped when the run ends.
`legacy-recall-before.log`: 2 new tests failed (first request waited 1.5 s and
injected there); `legacy-recall-after.log`: Cognee suite 295 passed, exit 0.

Improve and dream failed by timeout in most recorded attempts (dream 67 of 80,
other improves 51 of 72, each at exactly 30 s), and gating was per process, so
every short-lived process retried. A failure (other than cancellation) now
writes `<agent dir>/ice-cognee/improve-circuit.json`; automatic improve and
dream in any process, including at shutdown, skip for 5 minutes doubling to
1 hour, and a success clears it. `/cognee improve now` is not gated.
`improve-circuit-before.log`: a second runtime retried immediately (1 failed);
`improve-circuit-after.log`: 296 passed, exit 0. Timed-out mutations stay
`unknown`; B3 backend idempotency remains external. Documented in the user
guide.

**Review split and combined verification.** The work was committed as a local
snapshot (`backup/harness-hardening-snapshot-2026-11-04`) and cut into nine
branches whose union is byte-identical to it. Each branch alone passed format,
the warnings-denying workspace check, embedded-doc sync and its own suites
(`split/summary.txt`). The combined tree passed every CI gate
(`combined-gates.status`): workspace tests **3,627 passed, 0 failed,
14 ignored**, 96 summaries. The local observability database was scrubbed with
owner approval after a mode-600 backup: 17 values in 11 rows masked, none left.

**Final verification.** `final-gates.status` (offline, isolated agent dir):
format, warnings-denying workspace/all-targets check, embedded-doc sync and
clean diff, release-script tests and `git diff --check` all exit 0; workspace
tests/doctests **3,624 passed, 0 failed, 14 ignored** across 96 summaries.
`msrv-188-check.log`: `cargo +1.88 check -p ice-ai -p ice-cli --all-targets`
with warnings denied, exit 0. A full `x86_64-pc-windows-msvc` check cannot run
here (`windows-check.log`: `ring` needs MSVC); the verbatim
`termination_signal` function type-checks for that target against the locked
Tokio 1.53.1 with `deny(warnings)`. No Windows runtime verification yet: the
owner's Windows VM (`ssh kraken-vm`) was offline on 2026-11-04; rerun the
RPC/TUI signal probes and console-close check there when it is back. Public
API/format impact: none (internal functions, unchanged SQLite/session formats);
the binary entry point now bounds post-run blocking work to 500 ms.

Not addressed in this pass: H06, H11 implementation, H12, H13 remainder, H14
aggregate budgets, H16 HTTP/CLI exposure, H17/H18 (need human-reviewed labels
and owner-approved workloads). Also outside the plan: an uncommitted footer
keybinding-hint change (`ice-tui/src/footer.rs` and the removed
`footer.set_hints` call in `interactive_tui.rs`) and
untracked `web/observe-ui/{node_modules,.angular,dist}` build output; both
should stay out of the hardening commits.

## Active liveness/shutdown slice

- [x] Inspect observer/Cognee shutdown and H08/H09 ownership requirements.
- [x] Add payload-free terminal-persistence health and retry failed header writes.
- [x] Bound observer drain/finish/mirror using one deadline and truthful timeout reporting.
- [x] Reproduce Cognee task detachment; add cancellation-safe task ownership and component deadline.
- [x] Complete final deadline/error regressions and full workspace gates.
- [ ] Continue remaining recovery, liveness, memory, tool, and evaluation acceptance.

Observer `stop_until` shares an absolute Tokio deadline across forwarding, worker
completion and ClickHouse sync; the existing `stop` wrapper allows two seconds.
Cognee `session_shutdown_until` shares one deadline across helper/settle/background
drains, final improve and unregister; its wrapper also allows two seconds. These
are component budgets, **not yet one whole-CLI deadline**. No ACP grace increase
or session-history/schema migration is included. Both APIs are additive public
Rust APIs; plugin ABI and persisted formats are unchanged.

No local timeout can cancel an already-running synchronous OS write. The observer
abandons queued work, disables future finish and reports the active write as
unresolved instead of presenting a successful durable finish. Recorder health
uses atomics independent of the failed/blocked persistence mutex and contains
only counters/flags. Failed header writes do not poison deduplication. Terminal
and close failures remain distinct; repeated stop does not retry a terminal.

Cognee waits own abort-on-drop task batches, including handles temporarily removed
from registries. Shutdown closes task admission under each registry's lock,
cancels automatic/helper tokens and aborts pending async tasks even if its caller
is cancelled. Repeated settle notifications coalesce while one owned waiter is
active. Already-running blocking writes and accepted remote requests can still
have unknown outcomes: abort is not a rollback or an exactly-once guarantee.

Regression evidence under `target/harness-hardening-checks/`:

- `shutdown-observe-1.log`: compile exit 101 from deprecated `fetch_update` under
  warnings denial. Replaced with a saturating compare-exchange loop compatible
  with MSRV 1.88 rather than suppressing warnings or requiring newer APIs.
- `shutdown-observe-2.log`: 77 passed, 1 failed, 1 ignored, exit 101. Existing
  hung-mirror test exposed the outer deadline cancelling before the inner timeout
  recorded its diagnostic. Outer timeout now records the same payload-free
  diagnostic. Assertion and timeout budgets were not weakened.
- `shutdown-observe-3.log`: 79 passed, 0 failed, 1 ignored, exit 0, including
  cancellation of a stop caller, blocked sync writer, hung forwarder and terminal
  persistence failures.
- `shutdown-cognee-before.log`: exit 101, one selected regression failed with
  `cancelling the owner detached its background task`. The fixture explicitly
  aborts its probe even on failure. The fixed ownership makes cancelled waits
  abort owned tasks rather than detach them.
- `shutdown-cognee-1.log`: 108 passed, 0 failed, 2 ignored, exit 0.
- `shutdown-observe-4.log`: 80 passed, 0 failed, 1 ignored, exit 0, adding explicit
  queue-abandon coverage. Additional short mirror-deadline, repeated-settle and
  task-panic regressions also pass in the final workspace gates.

Final gates (`shutdown-gates.status`, `shutdown-final-*.log`) all exited 0:
format check; warnings-denying workspace/all-target check; warnings-denying
workspace tests and doctests (**3,605 passed, 0 failed, 14 ignored**, 94 summaries);
embedded-doc sync and clean generated diff; release-script tests (**4 passed**);
and whitespace check. Commands use offline dependencies, `ICERS_OFFLINE=1` and
an isolated agent directory. Failed intermediate logs are preserved. Branch and
baseline HEAD remain unchanged; no commits, pushes or publication occurred.

H08 remains open: stored `running` and recent activity still do not establish
process liveness. New provenance/instance ownership, conservative lease/liveness
presentation, crash/restart/clock/concurrent-process coverage and legacy-unknown
presentation remain necessary. H09 also needs the whole-CLI shared deadline,
EOF/disconnect/forced-kill and supported-platform verification. Component tests
and bounded caller return do not establish those acceptance criteria.

## Bounded local telemetry analysis-page prerequisite

- [x] Add a versioned, sanitized local-observe page contract and snapshot boundary.
- [x] Preserve unavailable-row markers instead of dropping malformed/oversize data.
- [x] Verify bounded pages, stable activity-update IDs and unchanged legacy bytes.
- [ ] Run final repository gates after the analysis-page API addition.

`ObserveRunStore::analysis_page(run_id, after, through, limit)` is a new public
Rust API, not yet an HTTP/CLI command. It returns schema version 1, source
`local-observe`, `unix_ms` timestamps, capture time, run summary, an append
sequence boundary, next cursor and has-more. Each page uses one SQLite read
transaction. Passing the first boundary excludes later appends; it does not
freeze historical mutations/retention between calls, join session/Cognee stores,
or establish a cross-store atomic capture. Supplied boundaries remain explicit
even if later retention removes rows. Missing runs return None.

Rows preserve sequence/time and typed activity item IDs; activity projections
replace by (run ID, item ID) in sequence order. Transport attempts, operations,
Cognee jobs and sessions are not inferred from text or merged into that identity.
Type mismatches, malformed JSON and >256 KiB source payloads return payload-free
markers. Limits are 1,000 rows and 2 MiB accounted source payload per page;
markers conservatively charge up to the per-row cap. SQL avoids loading oversize
payloads. These are not total allocator/encoded-output/RSS guarantees. Typed
payloads and summaries are re-sanitized on read; malformed source and decode
error text are not exported. Raw stored bytes remain unchanged.

The initial focused store suite (`analysis-store.log`) passed **14/0/0**; this is
new API behavior, not a claimed red-before-green defect in an existing analysis
export. Follow-up tests add byte-boundary pagination, stable activity/update IDs
and row-removal boundary semantics. H16 remains open: nested stringified lifecycle
metadata, complete provenance/attempt/unknown normalization, authorized HTTP/CLI
exposure, concurrent-process consistency stress and wider privacy acceptance.

## Activity-only dashboard presentation follow-up

- [x] Reproduce activity timestamps being labeled process `live`/crash-like `stale`.
- [x] Replace unsupported ownership claims with recent/quiet/unknown activity.
- [x] Keep recorded-status filters separate from recency and rebuild the embedded UI.
- [x] Run final repository gates after the dashboard rebuild.

`activity-ui-before.log`: UI unit tests **10 passed, 1 failed**, exit 1; recent
activity returned `live` rather than the requested activity-only `recent` label.
The formatter now distinguishes recent/quiet activity, recorded finished status,
and unknown status/invalid or backwards clocks. Neither activity recency nor
quietness claims a live/dead process. Sidebar/list/detail labels and tooltips no
longer infer crash/kill or say `never finished`. Run filters use recorded status:
a quiet running row is no longer silently included among finished runs. Detail
views outside the selected range no longer infer run completion from absence of
running items in a possibly truncated tape. Recent-activity polling remains an
activity heuristic; it is not an ownership check.

Cached `npm ci --offline` succeeded, reported zero vulnerabilities and warned
about five unapproved install-script packages (`@parcel/watcher`, `esbuild`,
`lmdb`, `msgpackr-extract`, `nx`). No install scripts were approved. UI formatting,
unit tests and `npm_config_offline=true sh scripts/build-observe-ui.sh` all
exited 0; source and generated embedded files are preserved together. Logs are
`activity-ui-{install,fmt,tests,build}.log`. No browser or platform verification
was performed. Final dashboard gates (`activity-ui-gates.status`) all exited 0:
**3,617 passed, 0 failed, 14 ignored**, 94 summaries, plus formatting,
warnings-denying workspace check, embedded-doc sync/diff, four release-script
tests and diff checks. UI tests: **11/11**. This presentation-only fix changes no
historical records, Rust DTO/schema, leases, heartbeats or process identity.
H08 remains open.

## Retained observer activity-byte bound follow-up

- [x] Reproduce count-only retained activity exceeding a declared byte budget.
- [x] Bound retained activity storage/string capacities and count evictions.
- [x] Verify Unicode pressure, growing terminal updates, oversize eviction and recovery.
- [x] Run final repository gates after activity-retention enforcement.

`retention-before.log` failed the exact Unicode pressure regression: 12 items
retained **28,807,212 accounted bytes**. The first context-mode call timed out
while its child test was still running; no duplicate was launched. Its completed
artifact contains the assertion and Cargo failure, not a successful verification.
The measurement-only baseline added accounting, but no retention enforcement.

`retention-observe.log`: **88 passed, 0 failed, 1 ignored**, exit 0; formatting
passed. The suite took **45.14 seconds** with the large-payload fixtures; this
is not evidence of a latency improvement, and redaction/serialization costs
remain to be profiled independently. No heap/RSS measurement was performed.
Final activity-retention gates (`retention-gates.status`) all exited 0: **3,617
passed, 0 failed, 14 ignored**, 94 summaries, plus formatting, warnings-denying
workspace check, embedded-doc sync/clean diff, four release tests and diff checks.

The live recorder now trims oldest activity items to **8 MiB** and 500 items,
including after tool/memory updates that grow existing items. Accounting includes
`size_of::<ObserveItem>()` per retained item plus every owned string's capacity,
including thread identity and run ID. `retention_health()` reports retained bytes
and a saturating eviction count; the first eviction emits a payload-free stderr
notice. Eviction does not mark a run/tool successful, alter authoritative session
data, or claim durable persistence failed. Durable event writes and live fan-out
remain independent of this bounded activity projection.

This is a provisional activity-item bound, not a process-heap/RSS guarantee. Deque
spare slots, allocator overhead, thread/lane/transition/dedup maps, broadcast/SSE
copies, transient serialization/redaction/input data, durable replay and many-run
pressure are outside it. Eviction counters are available on the recorder API,
not yet projected in historical snapshot schemas or the dashboard. H15 remains
open pending these surfaces, fairness, measured overhead and pressure baselines.

## Legacy memory acknowledgment truthfulness follow-up

- [x] Reproduce legacy false acknowledgment being mislabeled as append failure.
- [x] Map ambiguous legacy false to `rejected_unknown` without changing typed reasons.
- [x] Verify all eight typed outcome telemetry mappings and unknown delivery.
- [x] Run the Cognee runtime suite with warnings denied.
- [x] Run final repository gates after the telemetry label correction.
- [x] Upgrade the harness append-failure fixture to assert typed reason and repeat-run safety.
- [x] Run final repository gates after the harness fixture upgrade.

`legacy-ack-before.log` selected one test, exit 101: the boolean false path emitted
`append_failed`, although it cannot distinguish rejection from append failure.
It now serializes its existing derived outcome enum (`RejectedUnknown`) instead
of inventing a reason. Legacy outcome version 1 and byte/token fields remain;
future events change this ambiguous outcome string, but historical observations
are not rewritten. The typed version-2 path preserves persisted, duplicate,
empty, budget-rejected, cancelled, stale, append-failed, and rejected-unknown
reasons, zero persisted bytes for non-persisted outcomes, and `delivery: unknown`.
`ack-cognee.log`: **119 passed, 0 failed, 2 ignored**, exit 0; formatting passed.

Final acknowledgment gates (`ack-gates.status`) all exited 0; workspace tests/
doctests totaled **3,615 passed, 0 failed, 14 ignored**, 94 summaries. Formatting,
warnings-denying check, docs sync/diff, release tests, and diff checks passed.

The existing harness `RejectMemoryStorage` fixture now checks the typed
`AppendFailed` outcome twice, two completed foreground runs, absence of rejected
memory in both provider contexts, and absence from durable session entries.
`typed-append-tests.log`: **18/0/0**, exit 0; formatting passed. This extends an
existing before-write fault test without adding a wrapper or production change.
Final fixture gates (`typed-append-gates.status`) all exited 0: **3,615 passed,
0 failed, 14 ignored**, 94 summaries, plus formatting, warnings-denying workspace
check, docs sync/diff, four release-script tests, and diff checks.

The telemetry mapping tests do not prove that every harness race yields that
reason or that a backend accepted a mutation. The harness fixture proves the
before-write append rejection case only. Append-after-commit uncertainty,
stale/cancel races and remote accepted-work IDs still need integration evidence.
H10 and backend acceptance remain open.

## Observe server cancellation/deadline prerequisite

- [x] Reproduce caller cancellation detaching the server task.
- [x] Retain task ownership through close waits; abort on drop/timeout.
- [x] Add cancellation, hung-task deadline, direct-drop, and task-panic regressions.
- [x] Run focused observer suite and warnings-denying workspace check.
- [x] Run final repository gates after server-close change.
- [x] Share the app-level server/recorder cleanup deadline and test independent outcomes.
- [x] Run final repository gates after app-level observability integration.

`server-close-before.log` selected one test, exit 101: cancelling `close()`
detached the pending server task. `ObserveServer::close_until(deadline)` now keeps
its handle in `self` across the wait, reports false on timeout or join failure,
and uses `Drop` to cancel/abort owned work. Existing `close()` remains a wrapper
with a two-second component budget and a payload-free incomplete warning.
Connection tasks are owned by the server's `JoinSet`; dropping the server task
also aborts that set. An abort request is not proof a synchronous OS operation
has stopped or a durable write has completed.

`server-close-observe.log`: **85 passed, 0 failed, 1 ignored**, exit 0.
`server-close-check.log`: workspace/all-targets check with warnings denied,
exit 0. Tests cover cancellation during close, deadline expiry, direct drop,
task panic, and existing HTTP/SSE functionality. Server-state full gates
(`server-gates.status`) all exited 0; workspace tests/doctests totaled
**3,613 passed, 0 failed, 14 ignored**, across 94 summaries.

App cleanup now uses `ObserveRuntime::stop_with_server_until`: dashboard close
and recorder drain execute concurrently under one absolute observability deadline,
with separate server completion and recorder/persistence outcomes. A synthetic
hung server times out while the recorder durably finalizes with zero failed
writes. `shared-observe-tests.log` initially exited 101 on an agent fixture setup
error: a missing agent directory intentionally returns a warning, contrary to the
new test's setup assertion. This was not a reproduced shutdown defect. The
fixture now supplies an isolated temporary store beneath the verification root;
`shared-observe-tests-2.log` passed **86/0/1**, exit 0. Formatting and workspace
check also passed. This preserves the failed setup log rather than overwriting it.
Final integrated observability gates (`shared-observe-gates.status`) all exited 0:
workspace tests/doctests **3,614 passed, 0 failed, 14 ignored**, 94 summaries;
formatting, warnings-denying check, docs sync/diff, release tests, and diff checks
passed. These totals precede the legacy-acknowledgment mapping follow-up.

These are additive public deadline APIs and bounded wrapper behavior changes.
Adding `Drop` also prevents moving owned fields out of `ObserveServer` by value;
all workspace targets compile. No schema/format migration. This is not whole-CLI
integration: session plugin lifecycle has native-code timeout/keepalive safety
semantics that must be preserved, modes dispatch shutdown before returning to
app cleanup, and Cognee, session hooks, shells, native handlers and transport
still do not share this observability deadline. H09 remains open.

## SQLite takeover and process-termination extension

- [x] Inspect existing lease tests before extending recovery coverage.
- [x] Exercise recovery after expired-lease takeover at five write boundaries, before/after each write.
- [x] Kill a JSONL-owning child after acknowledged progress; recover and reopen twice.
- [x] Kill recovery before/after all five append boundaries and verify repair twice.
- [x] Rerun final gates after adding the boundary-kill matrix.

First full gate attempt: formatting, warnings-denying check, embedded-doc sync/
diff, release-script tests, and diff whitespace checks all exited 0. Workspace
run `lease-final-workspace.log` exited 101: **3,607 passed, 1 failed, 14 ignored**
across 94 summaries. The failure was
`oauth_codex::tests::callback_is_loopback_only_bounded_and_cancellable`, immediate
`TcpListener::bind(address)` assertion at `oauth_codex/tests.rs:510`. Five exact
reruns each selected one test and passed without callback-code changes
(`lease-oauth-exact-{1..5}.log`). Root cause remains unresolved; neither a baseline
defect nor a harmless flake is established. Full rerun
`lease-final-workspace-rerun.log` passed **3,608/0/14** across 94 summaries, exit 0,
before the additional boundary-kill test was added. The earlier self-update
`ETXTBSY` failure also remains unexplained.

`lease-recovery.log` passed 9 tests, exit 0. The new SQLite test uses a manual
clock, retains the stale facade, rejects takeover before expiry, advances beyond
the lease, rejects stale-owner recovery, retries an injected recovery write error
under the new fence, and verifies stale close cannot release the new owner's
lease. Two fully closed/reopened repositories preserve one assistant salvage,
one unknown/error result per unsafe call, and one terminal. The matrix covers
five boundaries both before and after delegated writes. Existing lease tests
already cover takeover fencing and heartbeat renewal; this extends them through
harness recovery rather than replacing those tests.

`lease-crash-1.log` and `lease-crash-2.log` each passed 11 tests, exit 0. The two
child entry points are no-ops in the parent run. The crash parent launches the
selected child, waits for its marker after acknowledged progress, verifies owner
exclusion, kills/reaps it without Rust destructors, then repairs and disk-reopens
twice. RAII kills/reaps the child on assertion failure. Fixture data/logs remain
inside the synthetic temporary directory beneath the verification artifact root
and are removed on completion. No real conversation data is used.

Final boundary-state gates (`boundary-gates.status`): formatting, warnings-denying
workspace/all-targets check, workspace tests/doctests, embedded-doc sync/diff,
release-script tests, and diff whitespace all exited 0. Workspace totals were
**3,609 passed, 0 failed, 14 ignored**, across 94 summaries. These gates precede
the server-close changes below.

`boundary-recovery.log` passed **12/0/0**, exit 0. The additional parent test
kills ten children at all five recovery append boundaries, both before and after
the delegated append. Each reached marker is written only by the selected fault
boundary; if recovery completes instead, the child panics and the parent fails.
Each termination is followed by repair and two disk loads with exact salvage/
result/terminal counts. No child continuation is allowed after parent failure.

These tests add expired-lease recovery and actual process termination **after**
acknowledged progress and **between** recovery appends. They do not establish
termination inside an OS write, physical
ENOSPC/permission failure, power-loss durability, SQLite child-process crash
recovery, or Windows behavior. H07/B2 acceptance stays open. No additional
production persistence change or format migration was needed for these tests.

## Recovery ownership and disk-verification slice

- [x] Inspect existing JSONL/SQLite ownership and reproduce shared-storage facade race.
- [x] Serialize repair by storage-object identity without changing persistence formats.
- [x] Add real-JSONL before/after write faults, repeated reload, corruption, and process-lock fixtures.
- [x] Verify lock lifecycle tests, full gates, and final diff; record results below.

**Contract:** backend ownership excludes independently opened native storage
objects (JSONL sidecar lock; SQLite writer lease). Within one process, every
facade sharing the same storage `Arc` serializes recovery's entire read/repair/
finish sequence. Keys are storage-object identity, not session metadata IDs.
Weak lock entries retain no session or idle lock and are pruned on lookup.
Storage rebinding during a run or recovery remains forbidden by the caller
contract; ordinary concurrent writers are not made transactional by this lock.
Separate wrapper objects hiding one backend still require their owner's
coordination. No cross-process exactly-once tool execution is claimed.

**Reproduction:** `recovery-independent-before.log`, exit 101, one selected test
failed because independent facades produced five entries instead of four. The
existing forced yield at commit intent exposed duplicate salvage. After the
storage-identity lock, `recovery-independent-after.log` passed all four then
existing tests, exit 0. Both commands use `RUSTFLAGS='-D warnings' cargo test
-p ice-harness --test recovery_write_failures --locked --offline`; the before
command additionally selects `independently_constructed_facades_share_recovery_ownership`.

**Disk fixtures:** `recovery-disk-1.log` passed eight tests, zero failed/ignored,
exit 0. Five recovery-write boundaries each fail before and after the delegated
JSONL write, then drop every storage handle and reload twice. Assertions cover
assistant text, paired unknown/error results, no duplicated salvage, and one
terminal. Native child-process probes test exclusion before and after atomic
torn-tail replacement and release after owner drop. Non-final syntax and final
schema corruption preserve original bytes, fail twice, and release ownership.
The restored files are synthetic fixtures only, not a data-repair policy.
All disk fixtures live beneath `target/harness-hardening-checks/` and clean up.

The eight-test count includes a child-probe entry point which is a no-op in the
parent run; the ownership test explicitly invokes it in three child processes
and requires one selected passing test each. This is real file reload/process
locking evidence with injected API write errors, **not** physical power-loss,
ENOSPC, permission-denial, SIGKILL, SQLite lease-expiry, or Windows evidence.
H07 and B2 therefore remain partially verified, not accepted. No storage schema,
native plugin ABI, or public Rust API change is introduced by this slice.

Additional focused checks: `recovery-locks.log` passed 2 selected unit tests,
zero failed/ignored (270 filtered out), exit 0: storage identity separates same-ID
sessions, shared facades exclude each other, idle locks are released, and rebinding
selects the replacement's lock. `recovery-disk-2.log` passed 8, exit 0. After
strengthening the independent-facade test with the full repaired-history helper,
`recovery-disk-3.log` again passed 8, exit 0; `recovery-assert-fmt.log` passed.

Initial full gates: format, warnings-denying workspace check, embedded-doc sync
and clean generated diff, release-script tests, and whitespace all exited 0
(`recovery-gates.status`, `recovery-final-*.log`). Full workspace tests exited
101: 3,589 passed, 1 failed, 14 ignored across 94 summaries. Failure:
`updates::tests::direct_update_replaces_the_selected_target_and_rejects_downgrades`
reported `replacement icers version check failed: spawn failed: Text file busy
(os error 26)` at the synthetic replacement executable. This is not a recovery
assertion failure, and is not classified as a confirmed baseline defect or a
proven flake from one occurrence. The original `recovery-final-workspace.log`
remains preserved. No update behavior or assertion was changed to bypass the
error.

Final workspace rerun (`recovery-final-workspace-2.log`, same warnings-denying
offline command) exited 0: **3,590 passed, 0 failed, 14 ignored**, 94 summaries.
The first exact-test rerun command incorrectly put `--exact` before Cargo's `--`:
`recovery-update-rerun.log` exited 1 with `unexpected argument '--exact'`; no test
ran, so this is an agent command error, not a product failure. Corrected command:
`RUSTFLAGS='-D warnings' cargo test -p ice-cli --lib --locked --offline
updates::tests::direct_update_replaces_the_selected_target_and_rejects_downgrades
-- --exact` passed 1, zero failed/ignored, exit 0
(`recovery-update-exact-2.log`). Passing reruns do not prove the cause of the
original transient filesystem error. Release-script verification passed 4 tests;
no compiler warnings/errors were emitted in the successful workspace rerun.
All synthetic disk fixture directories were removed on completion. Final
whitespace verification remains clean.

## Current blocker-removal pass

- [x] Revise delivery milestones, measurable acceptance, and dependency ownership.
- [x] Diagnose and repair the seven reproduced baseline test failures without weakening assertions.
- [x] Verify current implementation with full offline gates and review changes.
- [x] Record remaining external dependencies and actionable fallback paths.

The revised plan separates M0–M4 implementation/integration/acceptance, adds a
recovery/liveness/shutdown contract gate, and records B1–B6 ownership and safe
fallbacks. This does not close any H-item by implication.

Baseline failures were stale fixture-budget assumptions: output headroom and
read-tool schemas triggered preflight compaction before the intended boundary.
The exact-85% fixture now reserves only 100 output tokens and asserts its actual
850-token context/threshold; the mid-run summary failure fixture similarly
admits its first request. Post-tool extension fixtures use a 16,000-token window;
tiny-window budget fixtures remain unchanged. All original behavioral assertions
remain. Complete targeted suites passed 28 + 18, zero failed/ignored, exit 0
(`blocker-fixtures-1.log`, before the added explicit equality assertion).
Command: `RUSTFLAGS='-D warnings' cargo test -p ice-harness --test
coding_compaction_policy --test harness_extensions --locked --offline`, with
isolated agent directory and offline mode. Source: baseline HEAD above plus the
preserved dirty hardening tree and fixture edits. No production budget policy
was relaxed. Full final gates passed, including the added equality assertion.

Reviewed `capability_service.rs`: its legacy acknowledgment belongs to capability
hints, not a Cognee forwarding adapter. The typed hook's default boolean bridge
preserves its behavior; no forwarding patch is needed.

Queue closure follow-up reproduced oversized sends being accepted after close
or receiver drop (`queue-close-before.log`). Closed-state checking now precedes
loss accounting under the bookkeeping lock; payload measurement remains outside
the lock. Concurrent producer accounting and terminal reserve remain focused
fixtures, not retained-memory or shutdown-deadline acceptance.

Focused follow-up: memory acknowledgment tests passed 9; harness acknowledgment
unit/integration tests passed 1 + 1 after correcting the new fixture's Tokio
runtime; observer tests passed 72 with 1 ignored; spill tests passed 7. The first
cancellation regression failed due to reading observations before flushing, not
because it demonstrated the expected mislabel. That attempt is not counted as
failing-before evidence. Typed acknowledgment is an additive public Rust API;
it does not change the native plugin ABI or session storage format.

## Final blocker-pass verification

Source: `2fcc1618c0e81aec3e465c9fb24669a0d5655d84` plus the uncommitted
hardening changes and fixture corrections on `fix/harness-audit-hardening`.
All logs below are under
`/home/mewtwo/Zykairotis/ice-rust/target/harness-hardening-checks/`.
Environment: `ICERS_OFFLINE=1` and `ICERS_CODING_AGENT_DIR` set to that
directory's isolated `agent/` child. No live provider/configuration was used.

| Command | Result | Log |
| --- | --- | --- |
| `cargo fmt --all -- --check` | exit 0 | `blocker-fmt.log` |
| `RUSTFLAGS='-D warnings' cargo check --workspace --all-targets --locked --offline` | exit 0, no compiler warnings/errors | `blocker-check.log` |
| `RUSTFLAGS='-D warnings' RUSTDOCFLAGS='-D warnings' timeout 1800 cargo test --workspace --locked --offline --no-fail-fast` | exit 0; 3,582 passed, 0 failed, 14 ignored across 93 summaries | `blocker-workspace.log` |
| `sh scripts/sync-embedded-docs.sh` | exit 0 | `blocker-docs.log` |
| `git diff --exit-code -- crates/ice-cli/embedded-docs` | exit 0, no generated-doc diff | `blocker-docs-diff.log` |
| `python3 scripts/test_prebuilt_release.py` | exit 0; 4 passed | `blocker-release.log` |
| `git diff --check` | exit 0 | `blocker-diff.log` |

`blocker-gates.status` preserves per-command exit statuses. Formatting was
applied before verification (`blocker-format.log`, exit 0). The workspace log
includes the expected `register body exploded` panic diagnostic from the ABI
fixture: `loader::tests::panicking_register_body_is_contained_and_skipped`
passed, and its suite passed 137 tests. No failure was discarded. The corrected
compaction and extension suites passed 28 and 18 respectively in the full run.

Reviewed the fixture diff: assertions were preserved, equality setup was
strengthened, and production admission limits were not changed. Existing
headroom/schema tests remain enabled. The new plan does not authorize resolving
remaining design questions by weakening invariants. Local Linux gates are green;
Windows, real-disk/process recovery, shutdown bounds, retained observer-memory
pressure, backend/host authority, and human semantic evaluation remain pending.
B1 is resolved for this tree; B2–B6 retain their stated ownership and fallbacks.
This closes the current blocker-removal pass, not the eighteen-item program.

## Progress

- [ ] H01 synthetic baseline and test accounting (first batch recorded; full baseline pending)
- [ ] H02 telemetry redaction (bare provider-shaped tokens added 2026-11-04; acceptance pending)
- [ ] H03 memory lifecycle
- [ ] H04 billing retry (assistant, HTTP and distant quota-reset cases implemented; acceptance pending)
- [ ] H05 request budgeting
- [ ] H06 provider contracts
- [ ] H07 recovery fault injection and repair
- [ ] H08 liveness (signalled exits now finish; instance identity/heartbeat pending)
- [ ] H09 shutdown (SIGTERM/SIGHUP graceful in RPC and TUI; whole-CLI deadline pending)
- [ ] H10 memory outcomes
- [ ] H11 bounded memory work
- [ ] H12 permissions
- [ ] H13 capability instructions
- [ ] H14 output budgeting
- [ ] H15 observer pressure
- [ ] H16 analysis export
- [ ] H17 compaction semantics (30 compactions labelled, owner spot-check pending; fix in PR #103)
- [ ] H18 task evaluation (6-task suite run; regression found and fixed in PR #96; latest-turn overflow open)
- [ ] Full CI gates and final review

Completion requires the plan's acceptance criteria, not merely an existing test
passing. Partial changes and external blockers will be reported explicitly.

## First batch

- Baseline regressions: AI 92 passed / 2 failed; observe 59 passed / 3 failed /
  1 ignored. These were expected assertion failures, not compilation errors.
- Added telemetry-only typed redaction for recorder subscribers and snapshots;
  retained HTTP/SSE/store defenses; legacy read/mirror payloads re-sanitize
  without rewriting databases. Added prefixed keys, cookies, escaped JSON and
  URL userinfo cases. Original session messages/execution inputs are unchanged.
- Completed memory becomes terminal; unknown phases have no state chip. Recent
  request IDs suppress duplicate terminals and late starts. This is bounded
  deduplication, not unlimited historical deduplication. Old persisted wrong
  state chips are not migrated; raw observations retain their original phase.
- Billing/quota exclusions precede transient status matching. Numeric code 1113
  alone is intentionally not treated as globally meaningful. Added zero-retry
  assertions at assistant and transport layers.
- Provider headroom includes tool arguments, schema bytes, UTF-8 text, images,
  thinking, and message framing. Overfull known contexts fail explicitly.
  Ordinary OpenAI requests now enforce this path too, after sampling overrides.
  The harness compaction estimator remains separate and is not weakened.
- AI focused suite: 310 passed at the intermediate checkpoint; observer suite:
  65 passed, 1 ignored. Later adapter/transport tests require a final rerun.
- One intermediate provider run was stopped because an HTTP fixture awaited a
  request rejected before send. Preserved the ordinary Responses 16-token floor
  when there is room and no hard authority; next full provider run passed.
- A workspace-root permission denial prevented `ctx_execute_file` from reading
  `/tmp` logs. Subsequent logs are under `target/harness-hardening-checks/`;
  no permission settings were widened or denial bypass attempted.

## Follow-up implementation and verification

- Corrected an agent-authored missing `Context` qualification in the new
  Anthropic test. The initial workspace check/test compilation failures are not
  passing verification. The corrected warning-denying workspace check passed
  (`check-2.log`). Workspace tests then failed two compaction assertions; these
  also reproduce in isolation (26 passed, 2 failed). Both failures reproduce on
  source archived from unchanged HEAD (`compaction-baseline.log`), establishing
  that they predate this branch.
- Reproduced quoted-value redaction leakage with synthetic passwords containing
  spaces, then masked complete quoted values, escaped JSON, and quoted cookies.
  Observer suite: 65 passed, zero failed, one ignored (`observe-5.log`).
- Reproduced unconditional todo instructions despite unavailable tools. Prompt
  now requires advertised/enabled todo, with durable/visible checklist fallbacks
  and no plugin installation. Five prompt tests passed (`prompt-1.log`). Broader
  H13 capability acceptance remains outstanding.
- Added synthetic H07 faults before/after commit intent, assistant append, each
  of two tool-result appends, and terminal publication. The initial regression
  failed against old behavior (`recovery-before.log`). Repair now uses existing
  frame commit intents and publishes the terminal only after successful salvage;
  write failures propagate. Result lookup is branch-local. A shared facade lock
  serializes cloned-session recovery; independently constructed facades/processes
  still require exclusive ownership by the storage owner. Existing recovery
  suites passed 45 tests; the new write-failure matrix, forced-interleaving
  concurrent reopen, and cross-branch result tests passed (3 tests). Real-disk
  fault injection and independent-facade ownership remain to be verified.
- Latest provider suite: 313 passed, zero failed (`ai-4.log`).
- Warning-denying workspace check and formatting passed before the queue change.
  Full tests with `--no-fail-fast`: 3,565 passed, 7 failed, 14 ignored across 94
  test summaries (`workspace-tests-3.log`). All seven reproduce on unchanged
  HEAD: two compaction failures and five extension failures
  (`compaction-baseline.log`, `extensions-baseline.log`). They are not fixed or
  hidden by this branch, so the workspace test gate remains red.
- H15 partial implementation replaces the unbounded observer command channel
  with a 512-event / 8 MiB accounted-byte queue (256 KiB per event), reserving
  64 slots / 1 MiB for terminal events. Adjacent session-info samples coalesce;
  losses produce a payload-free gap note. Close is out-of-band and accepted
  events drain before exit. Limits are provisional synthetic-test settings,
  not measured process-heap guarantees. Retained recorder/SSE bytes, pressure
  measurements, and shutdown deadlines remain outside this partial change.

- Extended privacy verification through real HTTP state/combined/history/replay
  and live SSE routes, preserving authoritative input. Added terminal-first,
  conflicting-terminal and unknown-phase lifecycle tests. Observer suite:
  70 passed, zero failed, one ignored (`observe-8.log`).
- H14 reproduced that text/detail spill artifacts bypassed binary-only disk
  quotas (`spill-before.log`), then charged every artifact against the existing
  100 MiB / 10,000-file session budget. Private mode-600 files and teardown
  remain unchanged. Quota exhaustion explicitly reports unsaved evidence; this
  is not aggregate per-turn token budgeting or guaranteed output recovery.

## Outstanding acceptance work

H02 still needs wider stress/remote-history coverage. H03 retains legacy wrong
chips and needs additional phase-order fixtures. H05 estimates are approximations,
not tokenizer guarantees; Codex intentionally omits its optional output field
when no limit is requested. H06–H18 remain incomplete. Recovery now has shared-storage independent-facade
serialization, real JSONL fault/reload fixtures, and native process-lock probes;
SQLite lease, forced-kill, actual I/O-failure, and Windows acceptance remain.
Capability/output/shutdown evaluations also remain.
No item is considered complete merely because its focused tests pass.

