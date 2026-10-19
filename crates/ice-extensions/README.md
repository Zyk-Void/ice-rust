# ice-extensions

[![crates.io](https://img.shields.io/crates/v/ice-extensions.svg)](https://crates.io/crates/ice-extensions)
[![docs.rs](https://docs.rs/ice-extensions/badge.svg)](https://docs.rs/ice-extensions)

The host side of the plugin system: discovers, loads and drives Rust `cdylib`
plugins, and bridges their tools and events into the runtime's native async
types.

The ABI contract itself lives in
[`ice-plugin-sdk`](https://crates.io/crates/ice-plugin-sdk); this crate is the
other half.

## Install

```toml
[dependencies]
ice-extensions = "0.3"
```

Most users never depend on this directly — the `ice` CLI uses it, and the
harness consumes its adapters through injection.

## What it does

- **Loading.** `load_one` loads a single library, `load_dir` scans a directory,
  and `load_session` resolves the whole set for a session. Entrypoints are
  resolved through the single `ice_plugin_register` symbol. Plugins must use
  the unified ABI; incompatible registrations are reported as load failures.
- **Registry.** `ExtensionRegistry` holds what plugins registered: tools,
  providers, handlers, renderers and resource-discovery callbacks, with a
  deterministic snapshot the runtime can read.
- **Tool bridge.** `PluginToolAdapter` implements the agent's `AgentTool` trait
  on top of a plugin's four-function lifecycle (`execute` → `poll` → `cancel` →
  `destroy`). The blocking `poll` loop runs on `spawn_blocking` inside the
  ambient runtime — the adapter does not own a runtime of its own.
- **Partial results.** Tool progress flows back through an unbounded channel and
  is forwarded to `on_update`, behind a `catch_unwind` trampoline so a panicking
  callback cannot unwind across the FFI boundary.
- **Lifecycle events.** Dispatch wrappers for data, empty and lifecycle events,
  a veto path that lets a handler refuse startup, and a tee emitter so a plugin
  can observe events without stealing them from the TUI.
- **Providers.** `PluggableProvider` and `ExtensionProviderHooks` let a plugin
  serve a model or patch request options. A plugin that registers
  `register_streaming_provider` emits per-chunk envelopes that the host maps onto
  `AssistantMessageEvent`s as they arrive, so a plugin provider renders
  progressively instead of appearing all at once when the response completes. The
  same request fn also receives a `ProviderStreamControl`, and a plugin that polls
  it stops generating on a cancelled run instead of streaming a response nobody
  will read — one registration, and the plugin decides whether to check.

## Crate dependencies

`ice-extensions` depends on `ice-plugin-sdk`, `ice-ai` and `ice-agent` only —
never on `ice-harness`. The harness receives these adapters as trait objects, so
the dependency graph stays acyclic.

## Where plugins are discovered

| Location | Scope |
| -------- | ----- |
| `.ice/extensions` | Project (preferred) |
| `~/.ice/agent/extensions` | Global |
| `--extensions-dir <path>` | Explicit, added by the CLI |

## Installing and developing

```bash
ice install my-extension                  # build from crates.io and install
ice install my-extension --path ../local  # local development
ice dev                                   # build + watch + live reload
```

## Docs

- Crate docs: <https://docs.rs/ice-extensions>
- [Extension authoring guide](https://github.com/Zyk-Void/ice-rust/blob/main/docs/extensions/authoring.md)
- [Plugin ABI contract](https://docs.rs/ice-plugin-sdk)
- [Ready-to-install extensions](https://github.com/ice-rust/ice-package)

## License

MIT. See [LICENSE](https://github.com/Zyk-Void/ice-rust/blob/main/LICENSE)
and [NOTICE](https://github.com/Zyk-Void/ice-rust/blob/main/NOTICE).
