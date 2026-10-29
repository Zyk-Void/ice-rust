# Judged prompt recall

## Design and privacy

`readPolicy: "judged"` is opt-in. The default stays `legacy`; write-side
judgment and M5.4 status are unchanged. The request boundary supplies the
original user prompt and the latest preceding assistant text to the existing
Cognee runtime. Only the first provider request in a user run judges recall.
Tool-loop requests do not repeat the gate.

Recall and TypeSafe judgment are polled concurrently. A completed recall is
held until the judgment arrives. Because retrieval starts before the judgment
returns, the prompt may already have reached Cognee when the gate answers
`none` or `session`: `judged` saves the wait and the injection, not necessarily
the Cognee request or the prompt's disclosure to it. The typed request has exactly two questions:

- Choice `needs`: `none` (standalone), `session` (current conversation already
  available to the model), `memory` (earlier sessions, decisions or history).
- Noul `incomplete`: whether the request is too underspecified to act on.
  This probability is recorded only; it never changes behavior.

Only a `memory` choice at confidence >= 0.75 admits recall. `none` and `session`
cancel unfinished retrieval and inject nothing, including cached evidence.
The request cancellation token controls both futures; cancellation and session
generation changes discard late results. Dropping an HTTP future cannot
promise that the remote server stopped processing it.

The read helper binds the same explicitly configured decision provider and
model as the existing write helper, but does not require selective writes.
It uses the existing typed DecisionClient and its quotas/cache behavior, not
a chat completion. Missing provider/model, disabled helper and failed binding
are nonfatal. No model/provider guessing or chat fallback is introduced.

In production TypeSafe receives the user prompt (maximum 8,000 UTF-16 units)
and a previous-assistant excerpt limited to 800 UTF-16 units before the
existing truncation marker, plus the typed question instructions. Existing
heuristic memory-text redaction is applied. It is not a guarantee that secrets
or personal data are removed. Inputs exceeding the helper's estimated
2,000-token limit abstain instead of silently changing the full prompt.
No entire transcript, Cognee results or tool output is sent by this gate.

## Fallback and latency

Abstention, low confidence, malformed output, timeout, errors or unbound
configuration use the existing Event `asks_for_history` rule for that prompt.
Explicit history admits recall; an ordinary prompt cancels it. `incomplete`
is never a request for clarification or a write authorization.

The existing helper deadline is 1.5 seconds. An outer recall-budget timer also
bounds gate waiting, so helper failure cannot extend the existing retrieval
budget. The hoped-for JEV latency is about 300 ms versus about 3 seconds for
Cognee retrieval; these are expectations, not measurements. Successful memory
admission costs approximately the slower of retrieval and judgment. A skip
still starts retrieval, so this saves user-visible waiting rather than
necessarily reducing server work or billing.

The existing prompt recall activity records decision and fallback reason,
JEV latency, `incomplete`, whether recall was cancelled or used, and retrieval
latency when completed. `/cognee doctor` shows binding and the latest gate
metadata. No quality claim follows from fake-provider control-flow tests.

## Local owner-label evaluation

Do not run the real runner as part of tests or preparation. Preparation calls
no service:

```sh
python3 -I scripts/prepare_cognee_judged_eval.py
```

It reuses the heldout parsers, mixed tool-envelope filtering and scrubber.
Allowed sources are Claude project histories, Codex session histories and the
existing least source. Live `~/.icers` is excluded under this task's stronger
constraint. `.ice`, symlinked roots/subdirectories/files, stores, credentials
and build directories are not evaluation sources. Sampling uses seed
20261028, at most three prompts per source/session, round-robin source/day
strata, and a fixed shuffled page order. Slash-only commands and prompts too
large for the gate are excluded. Missing timestamps are excluded.

Private files are exclusively created at mode 600 under
`target/judged-eval/` (directory mode 700). Open `label.html` locally. Each
screen displays the prompt and previous assistant excerpt, with None / This
session / Older memory / Unsure labels. Browser localStorage is best effort;
use Download to export `owner-labels.json` into the same private directory.
The sample hash binds exported labels to the sample. Session text and labels
must never be committed or published. The committed manifest contains only
HEAD, seed and content hashes of sample, page, runner, helper source and
preparation script. Its HEAD is the source snapshot at preparation time.

The ignored `cognee::judged_eval_tests::judged_owner_label_eval` requires all of:

- `ICERS_JUDGED_EVAL_ALLOW_REMOTE=1` (explicit future permission to call TypeSafe);
- `ICERS_JUDGED_EVAL_REQUEST_CAP` in 1..200, at least the labeled count;
- `ICERS_JUDGED_EVAL_PROVIDER` and `ICERS_JUDGED_EVAL_MODEL`;
- isolated agent configuration that explicitly binds that provider.

It calls `Helper::bind_read_gate -> judge_read_gate` exactly once per labeled
prompt, with no retries and the production deadline. Each outcome, including
failure, is immediately exclusively written in the private directory. An
existing outcome aborts the run rather than quietly retrying it. Use a fresh
reviewed private directory for a genuinely new run.

The confusion matrix retains Unsure rows and failure columns. Skip and
unneeded-recall rates use all labeled prompts as denominators; missed-memory
uses all owner-memory prompts, including failures. Counts are reported as
numerator/denominator pairs. Failures and Unsure are also reported separately.
Latency p50/p95 includes failures. Estimated time saved subtracts all gate
latency from skipped prompts times median recall latency; this is a
conservative approximation because production overlaps the calls.

If `ICERS_JUDGED_EVAL_OBSERVATIONS` names an explicitly approved log copied
into the private evaluation directory, successful `remoteWaitMs` values
provide the recall median. Otherwise the report explicitly assumes 3 seconds.
No live observability state is discovered. Private `results.json` contains all
outcomes. Only the reviewed `aggregates.json` may later be copied into committed
evidence. No aggregates are fabricated before a permitted evaluation runs.
