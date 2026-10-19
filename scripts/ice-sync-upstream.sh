#!/usr/bin/env bash
# Pull new upstream pi-rust commits through base -> rebrand. See FORK.md.
#
#   scripts/ice-sync-upstream.sh          # review, then update base and rebrand
#   scripts/ice-sync-upstream.sh --review # only show what upstream changed
#
# Merging rebrand into main is left to you: git switch main && git merge rebrand
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

[ -z "$(git status --porcelain)" ] || { echo "working tree not clean" >&2; exit 1; }
git config rerere.enabled true

git fetch -q upstream main
old=$(git rev-parse base)
new=$(git rev-parse upstream/main)
if [ "$old" = "$new" ]; then
  echo "base already at upstream/main (${old:0:7})"; exit 0
fi
git merge-base --is-ancestor "$old" "$new" || { echo "upstream rewrote history; inspect manually" >&2; exit 1; }

echo "== upstream commits ${old:0:7}..${new:0:7} =="
git log --no-merges --format='%h %s (%an)' "$old..$new"
echo
git diff --stat "$old" "$new" | tail -n 25
echo
echo "full diff (original rpi names): git diff ${old:0:7} ${new:0:7}"
[ "${1:-}" = "--review" ] && exit 0

# base: fast-forward only; it never diverges from upstream.
git branch -f base "$new"

# rebrand: merge upstream, prefer upstream text on conflicts (it is pre-rename,
# and rebrand holds no hand edits), then re-run the rebrand over the result.
git switch -q rebrand
if ! git merge -q --no-edit -X theirs base -m "Merge upstream pi-rust ${new:0:7} into rebrand"; then
  echo "conflicts -X theirs could not settle (likely modify/delete):" >&2
  git diff --name-only --diff-filter=U >&2
  echo "resolve, commit, then run scripts/ice-rebrand.sh and commit" >&2
  exit 1
fi
scripts/ice-rebrand.sh
git diff --cached --quiet || git commit -q -m "chore(ice): re-apply rebrand after upstream ${new:0:7}"
cargo check -q --workspace --all-targets

echo
echo "rebrand updated. Next: git switch main && git merge rebrand"
