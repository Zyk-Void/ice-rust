# Observability: run history and the observe dashboard

icers keeps a forensic history of every agent run and can serve it in a local,
read-only browser dashboard. This is the Rust port of ice's `observe` feature
(`ICE_PORTING.md` §4), extended with cross-run analytics, a rebuilt dashboard
and an optional ClickHouse mirror. The Cognee knowledge views are not ported
yet.

## Run history (always on)

Each `icers` process starts one *run* and records a projection of its live
event stream:

| Activity | Recorded as |
|---|---|
| User prompts and skill invocations (`/skill args`) | `message` / `skill` rows |
| Assistant replies; aborted or failed turns, even when empty | `message` rows, failed ones marked `error` |
| Tool calls | One `tool` row per call: arguments, final result, duration, error state and a count of streamed updates |
| Retries | `retry` rows (`attempt 1/3 in 2000ms`, refused retries) |
| Compaction | `compaction` start and finish rows |
| Goal, verifier and Subagents V2 state entries | `goal` / `subagent` rows, on child threads for subagents |
| Model, thinking level, context use, tokens, cost | The run header |

History is stored in `~/.icers/agent/observability.sqlite`, or under
`$ICERS_CODING_AGENT_DIR` when set:

- It is separate from session storage, and recording never changes a session.
- The file is created with mode `0600` and uses WAL with full sync.
- Runs older than 30 days, by start time, are deleted when the file is opened.
- Before an event is written, string values under keys such as `token`,
  `api_key`, `secret`, `password` and `authorization` are replaced with
  `[REDACTED]`. Bearer credentials and `key=value` assignments inside other
  strings are redacted too. This is a best-effort filter, not a guarantee.
  The live dashboard shows unredacted data.
- Each retained string is capped at 1,048,576 characters, with an omitted-count
  suffix. The live view keeps the newest 500 activities.
- If the database cannot be opened, icers prints
  `Warning: observability history unavailable: …` and runs anyway.

## The dashboard (`--observe`)

```bash
icers --observe          # http://127.0.0.1:4659
icers --observe=5000     # explicit port; `--observe 5000` does not take the port
```

The startup line prints the dashboard URL, including a token. In the
interactive TUI the URL also appears in the transcript.

- **Loopback only.** The server binds `127.0.0.1` and rejects requests whose
  `Host` is not `localhost`, `127.0.0.1` or `[::1]`.
- **Token on every route.** The token can come from `?token=` (the page then
  sets an `HttpOnly`, `SameSite=Strict` cookie named `icers_observe_token`), a
  `Bearer` header, or that cookie. It persists in
  `~/.icers/agent/observe.token` (mode `0600`).
- **Read-only.** Only `GET`/`HEAD` are served, and no route controls the agent.
- **Shared port.** If the port is taken, icers assumes another icers owns it,
  prints `(shared existing port)`, and keeps recording. That dashboard lists
  every run with activity in the last 10 minutes. icers does not verify who
  owns the port.

The default port (4659) and cookie name differ from TypeScript ice (4649,
`ice_observe_token`), so the two never share a dashboard.

### Using the dashboard

The dashboard is a single-page app (Angular with spartan/ui components) served
by icers itself. It loads nothing from the network, and a strict
Content-Security-Policy allows only its own script, styles and API. Links are
hash routes, so a view can be bookmarked or shared, e.g. `/#/runs/<run id>?view=files`.
The top bar picks the time range (24h, 7d, 30d, 90d), the history source when
ClickHouse is configured, refresh and light/dark theme. `Ctrl K` opens a
command palette for pages, recent runs and actions; `Ctrl B` folds the sidebar.

- **Overview:** headline numbers for the range (runs, replies, tool calls,
  tokens, cache hit rate, cost), each with its change against the previous range
  of the same length and a trend line. Below them:
  - tokens per time bucket, stacked by input, cache read, cache write and output;
  - tool calls per bucket, failed calls stacked on top;
  - tool performance: calls, failure rate and p50/p95/max latency on a log axis;
  - when the agents work: tool calls and replies by local weekday and hour;
  - activity by kind, and how replies ended (`stop`, `toolUse`, `aborted`, …);
  - replies, tokens and cost per model, and the most-changed files.
- **Runs:** every run in the range with duration, replies, tool calls and
  failures, tokens, cost, files changed and its top tools. Unnamed runs are
  labelled with the start of their first prompt. Search, filter by model or
  status, sort any column, and hide empty runs. A running run with no activity
  for 10 minutes is marked as never finished (amber), since its process most
  likely crashed or was killed. Tick two runs to compare them.
- **Run detail** (click a run):
  - *Timeline:* the activity tape with kind filters (keys `0`–`9`), search
    (`/`), an errors-only switch and `↑`/`↓` selection. A resizable
    inspector shows details, token use of a reply, input, output, raw payload and
    the diff of an `edit`/`write` call. "Copy event JSON" copies the row.
  - *Waterfall:* when each tool call ran, per agent thread, coloured by tool,
    with prompts and replies as markers. Idle gaps over 20 seconds fold into
    hatched breaks (switch off "Compress idle" for a linear axis); zoom up to 16×.
  - *Files:* every file the run changed, with `+`/`−` totals and each change
    as a unified or side-by-side diff with line numbers and word-level
    highlights. `edit` diffs come from the patch the tool recorded; a `write`
    is diffed against the previous write of the same file in the run, or shown
    as new content.
  - *Tokens:* tokens per reply, prompt size per reply (how full the context
    got), and a table of every reply with cache hit rate, cost and stop reason.
  - *Tools:* per-tool calls, failures, latency percentiles and total time.
  - *Conversation:* prompts and replies as a transcript.
  - *Export JSON* downloads the run state.
- **Compare:** two runs side by side: metric deltas, tool calls per tool,
  files changed by one or both, a word diff of the first prompts, and an aligned
  diff of the prompt and tool-call sequence.
- **Live:** the activity of every recently live run as it streams in, a context
  gauge and token totals for this process, and an activity pulse over the last
  1, 3, 5 or 10 minutes. `Space` pauses or resumes following new rows.

### HTTP routes

All routes need the token.

| Route | Returns |
|---|---|
| `/` | The dashboard page; `/main.js`, `/styles.css` and `/licenses.txt` are its assets |
| `/state` | The current run state (`ObserveState`) |
| `/state?view=all` | The combined state of all recently live runs |
| `/state?runId=<id>` | One stored run, reconstructed from history (local first, then ClickHouse) |
| `/runs?limit=<n>` | `{ "runs": [...] }`, newest first (1–10000, default 100) |
| `/replay/<id>`, `/replay?runId=<id>`, `/replay?sessionId=<id>` | `{ run, events }`; supports `after=<sequence>` and `limit=<n>` |
| `/events` | Server-Sent Events: `snapshot`, `runs`, `activity`, `activity_update`, `lane`, `thread`, `header`, plus a heartbeat every 15 s |
| `/api/meta` | Version, live run id, local retention, available history sources and ClickHouse mirror status |
| `/api/overview` | Totals (and the previous window's), time series, per-model, per-tool and per-file statistics, activity kinds, stop reasons and the weekday × hour heatmap |
| `/api/runs` | Runs started in the window, each with its totals, models, top tools and first prompt |

The `/api/` routes take `since` and `until` (epoch milliseconds; defaults: the
last 7 days for `/api/overview`, 30 days for `/api/runs`), and
`/api/overview` also takes `bucket` (milliseconds) and `tz` (the viewer's UTC
offset in minutes, for day buckets and the heatmap). `source=sqlite` or
`source=clickhouse` picks a backend; without it SQLite is preferred so recent
local events remain visible while the asynchronous mirror catches up. If the
preferred backend fails, the other configured backend is tried and the reason
appears in `sourceError`. Invalid or reversed time windows return HTTP 400;
timestamps must be between 1970 and 9999. These routes are icers additions;
TypeScript ice has none.

Payloads use the camelCase field names of the TypeScript `observe-recorder.ts`
types, so tools written against the ice dashboard can read them.

## ClickHouse mirror (optional)

SQLite keeps 30 days of history on each machine. To keep history longer,
collect it from several machines, or query it with SQL or Grafana, point icers
at a ClickHouse server.

**Privacy:** setting the ClickHouse URL opts in to uploading full persisted
observation payloads, including prompts, tool arguments/results and assistant
messages. Credential-pattern filtering is best effort, not content anonymization;
private source code and conversation text may remain. Use only a trusted server
and configure transport security and access controls as appropriate.

```bash
docker compose -f deploy/observe-clickhouse/compose.yml up -d   # local ClickHouse
export ICERS_OBSERVE_CLICKHOUSE_URL=http://127.0.0.1:8123
export ICERS_OBSERVE_CLICKHOUSE_USER=icers               # optional
export ICERS_OBSERVE_CLICKHOUSE_PASSWORD=...             # optional
export ICERS_OBSERVE_CLICKHOUSE_DATABASE=icers           # optional, default icers
```

- **What is copied.** Each icers process creates the database and the
  `observe_runs` and `observe_events` tables if needed, copies every run that
  ClickHouse is behind on, then copies its own run every 2 seconds and once more
  when it exits (waiting at most 3 seconds). Events are copied exactly as stored
  in SQLite, so ClickHouse only receives redacted text.
- **Delivery is at least once.** Rows are keyed by `(run_id, sequence)` in
  `ReplacingMergeTree` tables, so a row sent twice is merged away, and every
  read ignores repeats.
- **Failures never block the agent.** An unreachable server is retried with
  backoff up to 60 seconds. The sidebar and `/api/meta` show whether the mirror
  is connected, when it last synced and the last error. A run recorded while
  ClickHouse was down is copied by the next icers process that starts.
- **Reading.** Analytics default to fresh local SQLite history. Select
  ClickHouse in the source picker for shared history or older runs; mirrored
  data can lag behind current activity. Individual runs older than local
  retention also open from ClickHouse. Remote replays over 100,000 events return
  an explicit error rather than silently displaying incomplete history.
- **Retention.** ClickHouse keeps everything; add a TTL yourself if needed,
  e.g. `ALTER TABLE icers.observe_events MODIFY TTL toDateTime(intDiv(ts_ms, 1000)) + INTERVAL 1 YEAR`.
- **Schema.** `observe_events` stores `run_id`, `sequence`, `ts_ms`,
  `event_type` and the JSON `payload`, plus columns ClickHouse derives at
  insert time (`kind`, `tool_name`, `state`, `duration_ms`, `path`,
  `usage_input`, `usage_output`, `usage_cache_read`, `usage_cost`,
  `turn_model`, `stop_reason`, …) for ad-hoc SQL. Read the latest state of
  each activity with `argMax(<column>, sequence)` grouped by
  `(run_id, item_id)`.

The URL must not embed credentials; use the user and password variables. An
invalid setting disables the mirror with a warning at startup.

## Working on the dashboard

The dashboard source is `web/observe-ui` (Angular, spartan/ui, TanStack
Charts). icers embeds the built bundle from `crates/ice-cli/src/observe/ui/`,
so building icers needs no Node. After changing the UI, rebuild and commit the
bundle:

```bash
sh scripts/build-observe-ui.sh                  # npm ci on first run, then ng build
cd web/observe-ui && npm test -- --watch=false  # unit tests
```

For live reloading, run `icers --observe` and then, in `web/observe-ui`,
`ICERS_OBSERVE_TOKEN=<token> npm start`. The dev server proxies the API to
`ICERS_OBSERVE_URL` (default `http://127.0.0.1:4659`).

## Differences from TypeScript ice

- Recording runs on its own thread, fed from the agent's broadcast stream, so a
  slow disk cannot stall the agent. If that stream overflows, the run records
  an `observe_note` row saying how many events were missed.
- The harness writes prompts and custom state entries straight to the session,
  so the recorder reads them from the session at run and turn boundaries.
  Entries that existed before the run are not replayed.
- Rust's harness reports compaction only as start and finish, so compaction rows
  carry no summary. There is no queue-update event yet, so `queue` rows do not
  appear.
- Reopening a stored run continues its event sequence, and pruning runs in one
  transaction.
- Combined views fold each foreign run's full history rather than its first
  250 events. Each SSE client has a bounded queue and gets a fresh snapshot if
  it falls behind.
- Only `/` and `/index.html` serve the page; TS also serves any `/index*` path.
- The dashboard is a different app from the TS `observe-dashboard.html`: it adds
  the overview, run list, run detail views, comparison and diffs above, and the
  `/api/` analytics routes behind them. The TS routes keep their payloads.
- The ClickHouse mirror has no TS counterpart.
