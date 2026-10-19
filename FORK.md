# ice-rust fork workflow

ice-rust is a fork of [bigfish1913/pi-rust](https://github.com/bigfish1913/pi-rust)
(the `rpi` Rust port of pi), rebranded to `ice` and grown toward parity with the
TypeScript [Zyk-Void/ice](https://github.com/Zyk-Void/ice) harness.

## Branches

| Branch    | Contents                                   | Rule                                   |
|-----------|--------------------------------------------|----------------------------------------|
| `base`    | Exact mirror of `upstream/main`            | Fast-forward only. Never commit here.  |
| `rebrand` | `base` + output of `scripts/ice-rebrand.sh` | No hand edits. Only merges + script output. |
| `main`    | `rebrand` + all ice work                   | Normal development. Merge `rebrand` in. |

Remotes: `origin` = `Zyk-Void/ice-rust`, `upstream` = `bigfish1913/pi-rust`.

Why this shape: the rename touches ~500 files, so every upstream change would
conflict with a hand-made rename. Keeping the rename as a deterministic script
means conflicts on `rebrand` are always resolved the same way: take upstream's
text, re-run the script. Your real ice changes on `main` only ever see upstream
changes that are already rebranded.

## Taking an upstream update

```bash
scripts/ice-sync-upstream.sh --review   # read upstream commits + diffstat, rpi names
git diff base upstream/main             # full upstream diff, if you want it
scripts/ice-sync-upstream.sh            # ff base, merge into rebrand, re-run script, cargo check
git switch main && git merge rebrand    # bring it into ice; conflicts here are real ones
cargo test --workspace
git push origin base rebrand main
```

Reviewing on `base` keeps upstream diffs small and readable (no rename noise).
To take only one upstream commit (say, a performance fix) without a full sync,
rebrand the patch on the fly and apply it on `main`:

```bash
git show <upstream-sha> | perl -pe 's/rpi/ice/g; s/RPI/ICE/g; s/Rpi/Ice/g' | git apply -3
```

A later full sync then sees that change as already present.

`rerere` is enabled by the sync script, so a conflict you resolve once on `main`
is replayed automatically next time.

## What the rebrand changes

`rpi`/`RPI`/`Rpi` become `ice`/`ICE`/`Ice` in paths and text: crates (`ice-ai`,
`ice-agent`, ...), the `ice` binary, `~/.ice/agent`, `ICE_*` env vars. Repo URLs
become `Zyk-Void/ice-rust`. Kept as-is: `@juicesharp/rpiv-todo` (external
package), the `rpi.laofu.online` domain (no ice domain yet), and upstream
PR/commit links in `CHANGELOG.md` and `NOTICE`.

To change the mapping, edit `scripts/ice-rebrand.sh` on `rebrand`, reset
`rebrand` to `base`, re-run the script, and merge `rebrand` into `main`.

## Caveats

- Upstream's release workflows (release-plz, crates.io, winget, Homebrew) are
  inherited. GitHub keeps Actions disabled on forks until you enable them. Do not
  enable the publish jobs before deciding crate names: `ice-*` may be taken on crates.io.
- `ice-sync-upstream.sh` stops on modify/delete conflicts that `-X theirs`
  cannot settle; resolve them by hand, then re-run `scripts/ice-rebrand.sh`.
