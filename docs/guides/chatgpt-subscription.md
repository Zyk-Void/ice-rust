# Use your ChatGPT subscription

The built-in `openai-codex` provider uses your ChatGPT subscription through
Codex's Responses endpoint. It does not require an OpenAI API key or a plugin.
Subscription eligibility and usage limits are controlled by OpenAI; an API
billing account is separate from a ChatGPT subscription.

## Sign in

```bash
icers auth login --provider openai-codex
```

Open the printed authorization URL if your browser does not open automatically.
By default, the browser returns to `http://localhost:1455/auth/callback`.
If the callback port is occupied, or the browser is on another machine, paste
the full redirect URL or authorization code into the login prompt. The redirect
URL includes a short-lived code: do not share it or paste it into a chat.
Press Escape or Ctrl+C to cancel terminal login.

The interactive TUI's `/login` selector offers the same built-in provider.
Use its prompt for manual input rather than starting another terminal reader.
An installed extension that declares ownership takes precedence over the
built-in flow.

## Select a model

```bash
icers --list-models
icers --model openai-codex/gpt-5.5 -p "Say hello"
```

After you sign in, and on every `/reload`, `icers` asks ChatGPT which models
your account may use and caches the answer in `~/.icers/agent/codex-models.json`.
The `openai-codex` model list is then exactly that list: models your plan does
not include are hidden, and models OpenAI adds later appear on the next
`/reload` without a new `icers` release. Models that are not in the built-in
list are shown without cost data. Before the first successful refresh, or when
a refresh fails, the cached list (or, without a cache, the built-in list) is
used. `ICERS_OFFLINE=1` skips the refresh. Signing out removes the cache.
Text/image inputs and thinking levels come from the account's model list or the
built-in model metadata.

## Check your usage

`/usage` shows your real ChatGPT plan limits next to this session's totals:
each rate-limit window (for example the 5-hour and weekly allowances) as a
colored bar with the percentage used and the time until it resets, any extra
metered limits, and your credit balance. Bars turn yellow at 60% and red at
85%. The panel then lists this session's cache hit ratio, cost, all-history
token totals and estimated active context.

The plan limits are fetched live from ChatGPT (`/wham/usage`) each time you run
the command. Without a ChatGPT login the panel says so, and with
`ICERS_OFFLINE=1` it skips the request. A failed request is shown in the panel
and does not hide the session totals.

## Prompt caching

Every request carries the session id as its prompt-cache key (and, for the
subscription backend, the `session-id` and `x-client-request-id` headers), so
each turn reuses the cached prompt prefix from the previous turn. To keep the
cache effective, avoid changing the model, tools or system prompt in the middle
of a session. The first turn of a session is always uncached, so the ratio in
`/usage` starts low and rises. `ICERS_CACHE_RETENTION=long` asks providers for
longer-lived caching (24h for OpenAI Responses, 1h for Anthropic), `none` turns
caching off, and `short` is the default.

## Credentials and refresh

Credentials are stored under the `openai-codex` entry in
`~/.icers/agent/auth.json`, or the directory selected by
`ICERS_CODING_AGENT_DIR`. On Unix, credential files are owner-only (`0600`).
Never commit or share this file. Access tokens, refresh tokens, and account
metadata are not session content or public model metadata.

Requests refresh tokens when fewer than five minutes remain. Refresh is
coordinated with other processes using the same credential store. An imported
TypeScript-shaped entry can contain `accountId` and an expiry in epoch
milliseconds; the Rust reader normalizes expiry to seconds. Rust continues to
write epoch seconds in its separate `~/.icers` store; this is an import path,
not a store intended to be shared with the TypeScript installation.

```bash
icers auth check --provider openai-codex
icers auth refresh --provider openai-codex
icers auth logout --provider openai-codex
```

`auth check` is local-only: it does not refresh credentials or probe your
subscription. An expired token can report not ready even though the next
request can refresh it. Logout deletes local credentials; it does not revoke
your account's sessions at OpenAI.

If refresh fails, sign in again. Quota/rate-limit errors show plan and reset
information when supplied by the server; other provider errors keep their
original meaning.

## Transport scope

This integration uses SSE. WebSocket pooling, device-code login, compressed
requests, service-tier fast mode, and Codex grammar/tool-search extensions are
not implemented by this provider. Separate per-call text-verbosity,
reasoning-summary, tool-choice and prompt-cache-key overrides are also not
exposed: this path uses low verbosity, automatic tool choice and
session-derived cache identity. Codex does not apply arbitrary model
`samplingParams` overrides. Other providers retain their existing API-key or
extension-owned authentication paths.
