---
name: ice-release
description: Publish a release of the icers Rust workspace, including preflight checks, version bumps, Git tags, GitHub Releases, crates.io, installation channels, website deployment, and post-release merging of main and creation of the next version branch. Use when the user requests a new release, tagging, crate publishing, or release-process work.
---

# icers Release Process

## Goal

Routine releases use a Release PR: ordinary PRs (including fork PRs) do not change version numbers. After `main` is merged and CI passes, release-plz automatically prepares a unified version and CHANGELOG. When a maintainer merges the Release PR, CI automatically publishes nine crates, binaries for five platforms, Homebrew/Scoop updates, and a PR to Microsoft winget.

The implementation entry point is `.github/workflows/release.yml`; configuration and credentials are documented in `docs/maintaining/releasing.md`. Do not treat merging an ordinary PR as authorization to publish, and do not use release secrets in fork or PR events.

The Task commands below are retained for manual releases and failure recovery. Do not run them in parallel with an automatic release that is already in progress.

The standard release entry point for this project is the root `Taskfile.yml`:

```bash
task publish
```

It performs version updates, tests and validation, commits, tagging, pushing, GitHub Release asset handling, installation-channel updates, crates.io publishing, and website deployment in dependency order.

## Pre-release checks

1. Check the current branch, remotes, and working tree:

   ```bash
   git status --short --branch
   git remote -v
   git log -5 --oneline --decorate
   ```

2. Do not discard uncommitted changes without authorization. If the user asks to "commit everything and publish," inspect the diff first, run tests, and then make a single combined commit.
3. Check the version and tags:

   ```bash
   grep '^version' Cargo.toml | head -1
   git tag --sort=-version:refname | head
   ```

4. Preview the release plan without making changes:

   ```bash
   task publish -- --dry-run
   ```

## Common release commands

Automatically bump the patch version:

```bash
task publish
```

Specify a version:

```bash
task publish RELEASE_VERSION=0.3.16
```

Bump the minor or major version:

```bash
task publish RELEASE_BUMP=minor
task publish RELEASE_BUMP=major
```

Skip interactive confirmation (only when the user has explicitly authorized it):

```bash
task publish RELEASE_VERSION=0.3.16 -- --yes
```

Arguments after `--` are forwarded to `scripts/release.sh`. Available phase arguments:

- `--dry-run`: print the plan only
- `--yes`: skip crates/site confirmation prompts
- `--only a,b`: run only the specified phases
- `--from phase`: resume from the specified phase
- `--skip a,b`: skip the specified phases
- `--date YYYY-MM-DD`: set the changelog/release date

## Standard phases

The release script normally runs these phases in order:

1. `preflight`: require a clean working tree and an unused version/tag
2. `bump`: update workspace versions and internal dependencies
3. `changelog`: split the Unreleased changelog section
4. `docs`: synchronize embedded docs
5. `validate`: validate versions across all crates
6. `commit`: create the release commit
7. `tag`: create `vX.Y.Z`
8. `push`: push the branch and tag
9. `wait`: wait for GitHub Release binary assets and checksums
10. `channels`: refresh Homebrew/Scoop/winget/site manifests
11. `commit-channels`: commit installation-channel updates
12. `taps`: push Homebrew/Scoop channel updates
13. `winget`: create a winget PR
14. `crates`: publish crates in dependency order to crates.io
15. `site`: deploy the website

## Recovering from a failed release

Do not bump the version again; resume from the failed phase using the version that is already prepared. Not every phase can be rerun directly: if a `tag` already exists, do not recreate it. Retrying winget file PUTs requires the existing file's SHA.

```bash
task publish RELEASE_BUMP=none -- --from push --yes
```

Or:

```bash
task publish RELEASE_VERSION=0.3.16 -- --from channels --yes
task publish RELEASE_VERSION=0.3.16 -- --only crates,site --yes
```

## Lessons from release 0.3.16

- For a long-running release, write output to `.deploy/release-X.Y.Z-resume.log`. After it completes, check the exit code and logs for every phase; do not treat a command timeout as a successful test.
- `winget-submit.sh` may fail to create a branch because the fork does not contain a commit from upstream master, and may misleadingly report that the branch "may already exist." First check whether the branch actually exists with `gh api repos/<owner>/winget-pkgs/git/refs/heads/ice-X.Y.Z`. After updating the fork, create the branch using a valid commit SHA. If necessary, create it from the fork's master, but verify that the PR diff contains only the expected manifests.
- Updating an existing winget manifest with PUT requires its SHA; do not create a duplicate PR if one already exists. After winget succeeds, resume from the `crates` phase if needed.
- The script does not push the main repository branch again after `commit-channels`. At the end of the release, run `git push origin HEAD` to ensure the installation-channel commit is also on the remote.
- Version numbers in this skill are examples. Confirm the current workspace version, tags, and user request each time; do not reuse an example mechanically.

## Post-release merge and next branch

Assuming the release branch is `feature-0.3.16`, after the release succeeds:

```bash
# Update local remote-tracking references
git fetch origin

# Switch to main and fast-forward to the latest remote state
git switch main
git pull --ff-only origin main

# Merge the release branch
git merge --no-ff feature-0.3.16 -m "merge release 0.3.16"
git push origin main

# Create and push the next version branch from main
git switch -c feature-0.3.17
git push -u origin feature-0.3.17
```

If the repository convention is to use fast-forward merges, confirm that first, then omit `--no-ff`. If `main` has new remote commits, stop and resolve the situation before proceeding; do not force-push.

## Permissions and credentials

A release requires:

- `gh auth status` to show write access to the repository
- Git push permission
- A `cargo login` token for crates.io
- SSH access required for website deployment
- Permission to operate the Homebrew/Scoop/winget channels

Check:

```bash
gh auth status
cargo login --help
```

Never write tokens, API keys, or SSH private keys into the repository.

## Post-release verification

After the release, verify:

```bash
git status --short --branch
git tag --list 'v0.3.16'
git ls-remote --tags origin v0.3.16
git ls-remote --heads origin main feature-0.3.17
cargo install ice-cli --version 0.3.16
icers --version
```

If winget has only had a PR submitted, do not claim that the release is already available in the winget repository; wait for Microsoft repository review and merge.
