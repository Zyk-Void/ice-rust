# AGENTS.md — working on ice-rust

Rules and context for anyone working in this repository, human or AI agent.
Read this whole file before changing anything. `FORK.md` covers the upstream
sync mechanics, `ICE_PORTING.md` the feature gaps, and `CONTRIBUTING.md` the
inherited upstream conventions. Where they disagree, this file wins for this
fork.

## Language

- Use English for assistant replies and user-facing documentation.
- When work encounters non-English prose in a file, translate the prose into
  English as part of the same focused change. Preserve source meaning and keep
  code, commands, identifiers, configuration keys, URLs and intentional quoted
  user content unchanged unless they themselves are the translation target.
- Translate source documentation first, then regenerate any embedded copies with
  `sh scripts/sync-embedded-docs.sh`.

## 1. What this repo is

- **ice-rust** (`github.com/Zyk-Void/ice-rust`) is a fork of
  [bigfish1913/pi-rust](https://github.com/bigfish1913/pi-rust), a Rust port of
  the pi coding agent. It was rebranded from `rpi` to `ice`.
- **The goal** is feature parity with the TypeScript harness **ice**
  (`github.com/Zyk-Void/ice`, local checkout `~/Zks/ice`).
  `ICE_PORTING.md` is the gap map. `ASTRA_GAP_AUDIT_PROMPT.md` is the prompt
  for a deeper, exhaustive audit.
- **Status: not production-ready.** The workspace builds, and `ice-cli` tests
  pass. Most ice-only features (Subagents V2, plan/build modes, background
  shells, observability, Cognee, Blackhole, most providers) are not ported yet.

## 2. Hard rules (never break these)

1. **Never touch the TypeScript ice installation.** Do not read from, write
   to, or point anything at `~/.ice/`, a project's `.ice/`, the `ice` binary, or
   `ICE_*` env vars. The Rust build has its own identity:
   - binary **`icers`**
   - config **`~/.icers/agent/`** (`auth.json`, `models.json`, `settings.json`)
   - project dir **`.icers/`**
   - env **`ICERS_*`** (e.g. `ICERS_CODING_AGENT_DIR`, `ICERS_OFFLINE`)

   Do not `cargo install` this crate under the name `ice`, and do not alias
   `ice` to it.
2. **Never hand-edit the `base` or `rebrand` branches.** `base` is an exact
   mirror of upstream. `rebrand` is `base` plus the output of
   `scripts/ice-rebrand.sh`. All feature work goes on branches off `main`.
3. **Never force-push `base`, `rebrand` or `main`.** Never rewrite published
   history.
4. **Never commit secrets.** `auth.json`, API keys, tokens and `.env` files stay
   out of git (`.gitignore` covers `.env*` and `.venv/`). Before every commit,
   run `git diff --cached` and read it. Keep credential files at mode `600`.
5. **Do not publish anything.** No crates.io publish, no `v*` tag push (it
   triggers `release-binaries.yml`), no GitHub Release, no version bump in
   `Cargo.toml`. Releases need explicit maintainer approval first (§9).
6. **Do not run `icers install <plugin>`.** Companion plugins are resolved by
   name on crates.io. After the rename their names (`ice-todo`,
   `ice-subagents`, ...) are not owned by us or upstream, so a third party could
   publish a native library under those names (`ICE_PORTING.md` §11). This
   stays open until the package-name decision is made.
7. **Do not open PRs or issues against upstream** (`bigfish1913/pi-rust`)
   unless the user explicitly asks. See §7.4.
8. **Never review Markdown files with OpenCodeReview (OCR).** Exclude all
   Markdown files (including `.md` and `.markdown`, case-insensitively) from
   the selected review files before invoking OCR, in both workspace and
   commit-range reviews. Do not send Markdown diffs to OCR or rely on
   discarding Markdown findings afterward.
9. **Use a code-first allowlist for OCR reviews.** Select Rust source and
   tests under `crates/`, `packages/`, and `examples/`; executable scripts
   (`.sh`, `.py`, `.js`, `.mjs`, `.ts`) outside excluded directories;
   `Cargo.toml`, `Cargo.lock`, Rust toolchain and build configuration; and
   `.github/workflows/` configuration. Include test fixtures only when needed
   to understand a selected code change. Exclude `docs/ice-gap/` entirely
   (including audit JSON, CSV, snapshots, and historical helper scripts),
   other documentation, generated artifacts, and copied documentation.
   Apply this scope before invoking OCR, not just when filtering findings.
   Report code/config statistics separately from documentation and evidence.
   This is a review filter only: preserve whole commits when integrating.

## 3. Branch model

| Branch | Contents | Who changes it |
|---|---|---|
| `base` | Exact mirror of `upstream/main` | `scripts/ice-sync-upstream.sh` only (fast-forward) |
| `rebrand` | `base` + `scripts/ice-rebrand.sh` output, plus `FORK.md` and the two `scripts/ice-*.sh` | Only to change the rebrand mapping, see `FORK.md` |
| `main` | `rebrand` + all ice work: ports, docs, `AGENTS.md`, `ICE_PORTING.md` | Feature branches merged by PR |
| `feat/*`, `fix/*`, `docs/*`, `chore/*`, `refactor/*`, `test/*`, `perf/*` | One focused change each | You |

Remotes: `origin` = `Zyk-Void/ice-rust` (push here), `upstream` =
`bigfish1913/pi-rust` (fetch only).

A fresh clone needs the local branches and upstream remote:

```bash
git remote add upstream https://github.com/bigfish1913/pi-rust.git
git fetch upstream
git branch base origin/base && git branch rebrand origin/rebrand
git config rerere.enabled true
git config blame.ignoreRevsFile .git-blame-ignore-revs
gh repo set-default Zyk-Void/ice-rust   # IMPORTANT, see §7.1
```

### 3.1 Taking upstream updates

```bash
git switch main && git pull --ff-only
scripts/ice-sync-upstream.sh --review   # read upstream commits + diffstat (rpi names)
scripts/ice-sync-upstream.sh            # ff base, merge into rebrand, re-run rebrand, cargo check
git switch main && git merge rebrand    # resolve real conflicts here
# run the CI gates (§5), then:
git push origin base rebrand main
```

Review upstream changes on `base`, where the diff has no rename noise. To
take a single upstream commit (for example a performance fix) without a full
sync, apply it on a branch off `main` with the same mapping the script uses:

```bash
git show <upstream-sha> | perl -pe 's/CARGO_BIN_EXE_rpi\b/CARGO_BIN_EXE_icers/g; s/(?<![A-Za-z0-9_-])rpi(?![A-Za-z0-9_-])/icers/g; s/rpi/ice/g; s/RPI/ICERS/g; s/Rpi/Ice/g' | git apply -3
```

### 3.2 Changing the rebrand mapping

Follow `FORK.md` ("What the rebrand changes"). The output must be
deterministic: two fresh runs from `base` give the same `git write-tree`
hash. Verify that before committing. The script runs `cargo fmt` and
`scripts/sync-embedded-docs.sh` itself, so its output passes CI as produced.

## 4. Layout

| Path | Owns |
|---|---|
| `crates/ice-ai` | Provider types and streaming. Wire APIs: anthropic-messages, openai-completions, openai-responses, faux. |
| `crates/ice-agent` | Agent loop, steering/follow-up queues, abort, hooks |
| `crates/ice-tools` | Built-in tools (read, write, edit, bash, grep, find, ls) and the execution env |
| `crates/ice-harness` | Session tree, JSONL persistence, compaction, skills, prompt templates, system prompt |
| `crates/ice-cli` | The `icers` binary: args, config (`config.rs`), TUI app, RPC/remote, packages, auth |
| `crates/ice-tui` | Terminal UI library |
| `crates/ice-plugin-sdk` | Stable `#[repr(C)]` plugin ABI (entry `ice_plugin_register`) |
| `crates/ice-extensions` | Plugin loader, actions bridge, provider plugins |
| `crates/ice-telemetry` | Span/event contracts |
| `packages/provider-runtime` | C ABI bridge for provider packages |
| `examples/*` | SDK, tools, TUI and plugin examples (built in CI) |
| `crates/ice-cli/embedded-docs/` | Generated copy of docs. Edit the source docs, then run `sh scripts/sync-embedded-docs.sh`. |
| `web/observe-ui` | Source of the `icers --observe` dashboard (Angular + spartan/ui). Its built bundle in `crates/ice-cli/src/observe/ui/` is generated: edit the source, then run `sh scripts/build-observe-ui.sh` and commit both. |

When porting from ice, the TypeScript reference lives in
`~/Zks/ice/packages/*/src`. `ICE_PORTING.md` gives the exact source paths for
each gap.

## 5. Build, test and CI gates

CI (`.github/workflows/ci.yml`) runs on pushes and PRs to `main`, on Linux
and Windows. A PR is not ready until all of these pass locally:

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --locked
RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings" cargo test --workspace --locked
sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs
python3 scripts/test_prebuilt_release.py
```

Optional, not enforced by CI: `cargo clippy --workspace --all-targets --locked`.

Practical notes:
- **MSRV is 1.88**, the highest `rust-version` among locked dependencies
  (`image`, `time`, `icu_*`). Do not use newer language or std features. Test
  MSRV-sensitive and dependency changes with `cargo +1.88 check`.
- **Disk.** Build on the NVMe checkout (`~/Zykairotis/ice-rust`). On the SATA
  drive, `--all-targets` builds saturate IO. For fast loops, use
  `cargo check` and `cargo test -p <crate>`. `target/` grows past 10 GB.
- **Tests must not need the network or a live provider.** Use the faux
  provider (`crates/ice-ai/src/providers/faux.rs`) and in-memory or temp-dir
  environments. Never touch the real `~/.icers`. Set `ICERS_CODING_AGENT_DIR`
  to a temp dir, as the existing tests do.
- **New behavior needs a test.** A bug fix needs a regression test that fails
  before the fix.
- **Python hooks** on this machine may create a `.venv/` in the current
  directory. It is git-ignored. Delete it if it appears outside the repo.

## 6. Running icers locally

```bash
cargo build --release -p ice-cli --bin icers
alias icers="$HOME/Zykairotis/ice-rust/target/release/icers"
icers --list-models          # sanity check of ~/.icers/agent/models.json
icers                        # interactive TUI
icers -p "summarize README"  # one-shot
```

`~/.icers/agent` was seeded on 2026-10-19 from a copy of `~/.ice/agent`
(`auth.json`, `models.json`, and 5 settings keys). Import differences are
listed in `ICE_PORTING.md` §11a. Never sync back to `~/.ice`. If an import
needs redoing, copy from `~/.ice` again rather than editing TS ice files.

## 7. GitHub workflow

### 7.1 Before touching GitHub
- `gh` treats a fork's PRs as targeting the **parent** repo by default. Run
  `gh repo set-default Zyk-Void/ice-rust` once per clone, and pass
  `-R Zyk-Void/ice-rust` in scripts. Check before every `gh pr create`:
  `gh repo set-default --view` must print `Zyk-Void/ice-rust`.
- `git config user.name` / `user.email` must be the identity the user wants
  on commits.

### 7.2 Branches and commits
- Branch from an up-to-date `main`: `git switch main && git pull --ff-only &&
  git switch -c feat/<area>-<short-desc>`. Examples:
  `feat/tools-background-shells`, `fix/ai-ultra-thinking-level`,
  `docs/porting-subagents`.
- **One logical change per branch and per PR.** Do not mix refactors,
  formatting sweeps or drive-by fixes into a feature PR.
- **Conventional Commits:** `type(scope): imperative summary`, subject
  ≤ 72 chars, no trailing period. Types: `feat fix docs refactor perf test
  chore build ci revert`. Scope is the crate or area without the `ice-`
  prefix (`ai`, `agent`, `tools`, `harness`, `cli`, `tui`, `plugin-sdk`,
  `extensions`), or `ice` for fork-level changes.
- The body explains **why** and any behavior change, wrapped at about 72
  columns. Reference the gap: `Ports ICE_PORTING.md §3 (background shells).`
  Add `BREAKING CHANGE:` footers when they apply.
- Commits should each build and pass tests (bisectable). Squash "wip" and
  "fix typo" commits before review with `git rebase -i` on your own unpushed
  or unreviewed branch only.
- AI-authored commits carry a `Co-Authored-By:` trailer for the agent.
- Stage explicit paths (`git add <paths>`), not `git add -A`. Read
  `git diff --cached` first.

### 7.3 Pull requests (into `Zyk-Void/ice-rust` `main`)
- Push the branch: `git push -u origin <branch>`. Open the PR with
  `gh pr create -R Zyk-Void/ice-rust --base main`.
- Fill in `.github/PULL_REQUEST_TEMPLATE.md`: what and why, the affected
  crates, how it was verified (paste the gate commands and their results), and
  the TS ice source it ports, with paths.
- **Call out explicitly** any change to the plugin ABI (`ice-plugin-sdk`,
  affects every built plugin), to session persistence (`ice-harness`, describe
  crash recovery mid-operation), to on-disk formats under `~/.icers`, or to
  public crate APIs.
- Keep PRs reviewable: aim for under ~600 changed lines excluding tests and
  generated docs. Split large ports into stacked PRs per `ICE_PORTING.md`
  §12.
- Use draft PRs (`--draft`) for work in progress. Mark ready only when CI is
  green.
- Merge policy: **squash-merge** feature PRs, so there is one Conventional
  Commit per PR on `main`. Merge `rebrand` into `main` with a **merge commit**
  (never squash), so future syncs keep a merge base. Delete feature branches
  after merging.
- Resolve review comments with new commits during review. Never force-push
  over a reviewer's view of a shared branch without saying so.

### 7.4 Upstream contributions
A fix that is generic, not ice-specific, can go upstream so we stop carrying
it. Only do this when the user asks. Branch off `base` (not `main`), use
upstream's `rpi` names and `CONTRIBUTING.md`, and open the PR against
`bigfish1913/pi-rust`. The fix comes back to us through the normal sync.

### 7.5 Issues, CI and repo settings
- Track gaps as issues titled with the `ICE_PORTING.md` section, e.g.
  `[porting §3] background shells`. Labels: `porting`, `bug`, `security`,
  `upstream-sync`.
- Actions are enabled on the fork. CI runs on pushes and PRs to `main`. Fork
  workflows may need one-time approval in the Actions tab.
  `release-binaries.yml` runs on `v*` tags, so do not push tags.
  `release.yml` needs crates.io secrets the fork does not have, and that is
  intended.
- Recommended (owner action): protect `main`, `base` and `rebrand` in
  Settings, Branches, with no force-push and no deletion; on `main`, require
  PRs and the CI checks.
- Security problems go to the owner privately, not to public issues.

## 8. Porting work

1. Pick a gap from `ICE_PORTING.md` (§12 gives the order). Read the TS source
   it cites in `~/Zks/ice`, including its tests. The tests are the spec.
2. Put the code in the crate that owns the behavior (§4). Don't create new
   crates or abstractions without a concrete need.
3. Keep the TS on-disk and wire formats readable where ice defines them
   (sessions, `models.json`, settings), unless a difference is deliberate and
   documented.
4. Add faux-provider tests that mirror the TS tests. Update user docs and
   re-sync `embedded-docs`.
5. Update `ICE_PORTING.md`: mark the entry ported (with a PR link) or record
   what remains.

## 9. Releases (not yet)

No releases until the owner decides on: crate and plugin package names
(crates.io ownership, §2.6), the plugin ABI symbol, and whether `icers` stays
the binary name. Then follow `CONTRIBUTING.md` "Releases", using
`Zyk-Void/ice-rust` and our own secrets.

## 10. Current state and open decisions (2026-10-19)

- Branches pushed: `base` (upstream `0fb2d89`), `rebrand`, `main`.
- Verified on the NVMe checkout: rustfmt clean; `cargo check --workspace
  --all-targets` clean; `cargo test --workspace --locked --no-fail-fast`
  1927 passed, 0 failed, 5 ignored. Clippy (not a CI gate) reports only
  style lints, the most common being `type_complexity` and
  `field_reassign_with_default`. Leave them alone unless fixing them is the
  task.
- `~/.icers/agent` is seeded. `icers --list-models` loads all 8 providers from
  the imported catalog (479 omni models). The default is `omni/mec/mercury-3`.
- Open decisions: plugin package names and ABI symbol (§2.6, `ICE_PORTING.md`
  §11); adding the `ultra` thinking level (§11a); built-in providers for the
  `zai`/`opencode` keys (§11a).
