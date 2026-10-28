# Project documentation

Browse current usage and implementation documentation by feature. Version changes
are recorded in the [CHANGELOG](../CHANGELOG.md); future plans are in the
[ROADMAP](../ROADMAP.md).

| Feature | Documentation |
| --- | --- |
| Installation, commands, configuration, and everyday use | [User guide](guides/user-guide.md) |
| SDK layers and crate responsibilities | [Architecture overview](architecture/overview.md) |
| Provider boundaries, standalone packages, and migration | [Provider packages](architecture/provider-packages.md) |
| Provider integration, authentication, streaming, and acceptance criteria | [Provider integration](extensions/provider-integration.md) |
| Creating agents, reusing tools, and embedding the SDK | [Project structure](agent/project-structure.md) |
| Messages, streaming responses, and tool execution | [Agent loop walkthrough](agent/loop-walkthrough.md) |
| Long sessions and context compaction | [Context compaction](agent/context-compaction.md) |
| Rust dynamic-library extensions and ABI | [Extension authoring](extensions/authoring.md) |
| Adding or changing agent tools (built-in, plugin, MCP) and the gap versus ice | [Adding tools](guides/adding-tools.md) |
| Startup, session, and shutdown events | [Extension lifecycle](extensions/lifecycle.md) |
| Subagent configuration inheritance, permissions, and lifecycle | [Host-managed subagents](extensions/subagents.md) |
| Stdio MCP servers and tool discovery | [MCP tools](extensions/mcp.md) |
| Settings-driven lifecycle and tool commands | [Command hooks](extensions/hooks.md) |
| Context-efficient MCP tools and hook routing | [context-mode recipe](extensions/context-mode.md) |
| Headless server and remote TUI | [Remote mode](remote/user-guide.md) |
| Agent and extension debugging | [Rust debugging guide](debugging/rust-debugging.md) |
| Repeated reasoning, cancellation, and crash recovery | [Incident analysis](debugging/llm-repetition-forensics.md) (historical evidence and fixes) |
| Startup, memory, and installation-size measurements | [Performance benchmarks](performance/benchmark-vs-pi.md) (including measurement dates and scope) |
| Releases and cross-platform distribution | [Release process](maintaining/releasing.md) |
| Introductions and article drafts | [Promotion material](promotion/README.md) |
| Model-readable input for the CLI `docs` tool | [Embedded documentation sources](ice-tool/README.md) |

## Origins and acknowledgements

icers began as a Rust reimplementation of Mario Zechner's
[Pi TypeScript SDK](https://github.com/earendil-works/pi), and has developed its
own runtime and native extension ABI. See the root
[README](../README.md#acknowledgements), [NOTICE](../NOTICE), and
[LICENSE](../LICENSE) for origins, copyright, and licensing. A native Rust
implementation does not mean that upstream designs were not consulted, or that
Pi TypeScript/npm extensions can be loaded directly.

## Maintenance conventions

- Keep usage and implementation documentation in the appropriate feature directory;
  update it when behavior changes.
- `ice-tool/` is the separate model-readable documentation source. Synchronize it
  into the CLI's `embedded-docs/` with `scripts/sync-embedded-docs.sh`; do not edit
  only the generated snapshot.
- Maintain release records in the root `CHANGELOG.md`, rather than adding duplicate
  per-version release documents.
- Obsolete milestone issue records, gap tables, and old promotional drafts have
  been removed; they remain available in Git history.
- Keep dates and applicability boundaries for incident analyses and benchmarks;
  they are not lists of current missing features.
