# Observability: run history and the observe dashboard

icers keeps a forensic history of every agent run and can serve it in a local,
read-only browser dashboard. This is the Rust port of ice's `observe` feature
(`ICE_PORTING.md` §4). The Cognee knowledge views are not ported yet.

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

- **Activity tape:** live rows coloured by agent thread, with a search box and
  kind filters (`0` all, `1` tool, `2` message, `3` skill, `4` compaction,
  `5` retry, `6` queue, `7` subagent, `8` memory, `9` goal).
- **Inspector:** select a row to see metadata, pretty-printed arguments, text,
  the result, the raw payload and a diff for `edit`/`write` tools. "Copy event
  JSON" copies the row. On wide screens the inspector can be resized by dragging
  or with the arrow keys; on narrow screens it opens as a drawer.
- **Pulse:** activity density over the last 1, 3, 5 or 10 minutes in 42 bins,
  with one trace per agent when several runs are live. Click a bin to jump to
  its latest event.
- **Run picker:** switch between all live agents and a single recorded run,
  including finished ones.
- **Keys:** `Space` pauses or resumes auto-scroll, `↑`/`↓` move the selection,
  `Esc` closes the inspector.

### HTTP routes

All routes need the token.

| Route | Returns |
|---|---|
| `/` | The dashboard page |
| `/state` | The current run state (`ObserveState`) |
| `/state?view=all` | The combined state of all recently live runs |
| `/state?runId=<id>` | One stored run, reconstructed from history |
| `/runs?limit=<n>` | `{ "runs": [...] }`, newest first (1–10000, default 100) |
| `/replay/<id>`, `/replay?runId=<id>`, `/replay?sessionId=<id>` | `{ run, events }`; supports `after=<sequence>` and `limit=<n>` |
| `/events` | Server-Sent Events: `snapshot`, `runs`, `activity`, `activity_update`, `lane`, `thread`, `header`, plus a heartbeat every 15 s |

Payloads use the camelCase field names of the TypeScript `observe-recorder.ts`
types, so tools written against the ice dashboard can read them.

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
