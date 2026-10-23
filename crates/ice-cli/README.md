# ice-cli

[![crates.io](https://img.shields.io/crates/v/ice-cli.svg)](https://crates.io/crates/ice-cli)
[![docs.rs](https://docs.rs/ice-cli/badge.svg)](https://docs.rs/ice-cli)
[![CI](https://github.com/Zyk-Void/ice-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Zyk-Void/ice-rust/actions)

The terminal coding-agent CLI — the `icers` binary — built on the `ice-*` library
crates. It is the ready-to-run surface of the
[icers](https://github.com/Zyk-Void/ice-rust) project: an interactive TUI, a
one-shot `-p` mode, a JSON event mode, durable sessions, and Rust `cdylib`
plugin loading.

## Install

```bash
cargo install ice-cli
```

Requires Rust 1.88 or newer. Then point it at a provider and start it:

```bash
export ANTHROPIC_API_KEY=...     # or OPENAI_API_KEY, or ~/.icers/agent/models.json
icers
```

One-shot, and machine-readable output:

```bash
icers -p "summarize the README"
icers --mode json -p "list the crates"
```

## Authentication

Credentials are resolved in priority order:

1. `--api-key` on the command line
2. `~/.icers/auth.json`, written by `icers auth login` (mode `0o600` on Unix)
3. the `apiKey` field of a provider in `~/.icers/agent/models.json`
4. provider environment variables: `OPENAI_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
   `ANTHROPIC_API_KEY`

`~/.icers/` can be relocated with `ICERS_CODING_AGENT_DIR`.

## Custom providers and gateways

Any Anthropic-Messages or OpenAI-compatible endpoint can be declared in
`~/.icers/agent/models.json`:

```jsonc
{
  "providers": {
    "gateway": {
      "api": "anthropic-messages",
      "baseUrl": "https://gateway.example.com",
      "apiKey": "sk-gateway-secret",
      "authHeader": true,
      "headers": { "x-portkey-key": "…" },
      "models": [{ "id": "custom-claude", "name": "Custom Claude" }]
    }
  }
}
```

```bash
icers --model gateway/custom-claude -p "hi"
```

Use `"api": "openai-completions"` for an OpenAI-compatible endpoint. Anthropic
endpoint overrides also honour `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN`.

## Sessions and resources

- Sessions are JSONL v4 files under the session directory, with branching and
  context compaction.
- Project resources are discovered from `.icers/` (skills, prompts, themes, and
  `SYSTEM.md`), with `~/.icers/agent/` as the global layer. When both define the
  same skill or prompt name, the project copy wins.
- Plugins are loaded from project `.icers/extensions`, global
  `~/.icers/agent/extensions`, and `--extensions-dir`.

## Plugins

```bash
icers install ice-todo                           # prebuilt: repository discovered via crates.io
icers install ice-todo --version 0.2.0
icers install my-extension --github owner/repo --version 1.2.3
icers install ice-todo --build                    # explicitly compile source
icers install my-extension --path ../my-extension --force   # local
icers dev                                        # build + watch a local extension
```

`icers install` defaults to the versioned `ice-extension.json` prebuilt protocol: it resolves a non-yanked stable package version and repository through crates.io, loads the `<package>-v<version>` Release manifest, and verifies the archive and declared library before transactionally installing it into `~/.icers/agent/extensions`. Explicit `--github owner/repo` installs use package-specific tags and also support unpublished extensions. Both repository forms consume the same protocol; they never infer library filenames or use repository-wide latest bundles. Missing prebuilt releases fail with an actionable error; `--build` and `--path` explicitly select Cargo compilation. Installed provenance preserves the source for update discovery, reinstalls and uninstall. Actual plugin ABI/API-size validation still happens when loading the library.

`icers install <name>` installs or upgrades an extension without `--force`. An already installed version is skipped; use `--force` to reinstall the same version or allow a downgrade. These rules also apply to explicit Cargo builds and local installs.

`icers package update` updates all linked project packages and globally installed registry/prebuilt extensions; `icers package update <name>` selects one package. Updates retain the original Cargo build backend or explicit GitHub repository. Global development installs made with `--path` are excluded from registry updates.

## Remote mode

```bash
icers --server                      # headless agent over TCP, prints a token
icers --connect host:port --token T # zero-local-resource TUI client
```

The client holds no local provider, tools, extensions or session files; all
agent work happens on the server. See
[docs/remote/user-guide.md](https://github.com/Zyk-Void/ice-rust/blob/main/docs/remote/user-guide.md).

## Default tools

The CLI loads the Pi-compatible `read`, `write`, `edit` and `bash` tools.
Additional implementations (`grep`, `find`, `ls`, `docs`, `powershell`) remain
available in the [`ice-tools`](https://crates.io/crates/ice-tools) library and
through the `default_tools` setting.

## Library usage

The crate also exposes its internals, so the CLI surface can be embedded:

```toml
[dependencies]
ice-cli = "0.3"
```

For building your own agent, depend on
[`ice-agent`](https://crates.io/crates/ice-agent) and
[`ice-harness`](https://crates.io/crates/ice-harness) directly instead — see
[docs/agent/project-structure.md](https://github.com/Zyk-Void/ice-rust/blob/main/docs/agent/project-structure.md).

## Documentation

- User guide: <https://rpi.laofu.online/docs.html>
- Architecture: [docs/architecture/overview.md](https://github.com/Zyk-Void/ice-rust/blob/main/docs/architecture/overview.md)
- Extension authoring: [docs/extensions/authoring.md](https://github.com/Zyk-Void/ice-rust/blob/main/docs/extensions/authoring.md)
- Changelog: [CHANGELOG.md](https://github.com/Zyk-Void/ice-rust/blob/main/CHANGELOG.md)

## License

MIT. See [LICENSE](https://github.com/Zyk-Void/ice-rust/blob/main/LICENSE)
and [NOTICE](https://github.com/Zyk-Void/ice-rust/blob/main/NOTICE).

This crate is part of a Rust port of the MIT-licensed TypeScript `pi` SDK
(© Mario Zechner).

### CLI self-update

`icers update` downloads the matching GitHub prebuilt CLI using the versioned `ice-release.json` distribution protocol. It verifies target, archive size, both SHA-256 hashes, and the executable version before using the existing atomic replacement and Windows staging helper. Version discovery uses crates.io. Only `icers update --build` selects local Cargo compilation; missing manifests and failed checks preserve the installed executable. `--force` reinstalls and `--offline` reports cached information without downloads or builds.
