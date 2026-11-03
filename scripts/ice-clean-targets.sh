#!/usr/bin/env bash
# Reclaim disk from cargo `target/` dirs of every git worktree of this repo.
#
#   scripts/ice-clean-targets.sh                    # dry run: list sizes, delete nothing
#   scripts/ice-clean-targets.sh --apply            # delete target/ of linked worktrees
#   scripts/ice-clean-targets.sh --incremental      # only drop debug|release/incremental (cheaper rebuild)
#   scripts/ice-clean-targets.sh --older-than 3     # only worktrees untouched for 3+ days
#   scripts/ice-clean-targets.sh --include-main     # also clean the primary checkout
#
# Worktrees come from `git worktree list`, so ones outside the repo
# (~/.t3/worktrees, /tmp) are covered. A worktree with a running cargo or
# rustc is skipped. Only build output is removed; sources and uncommitted
# changes are never touched. Targets regenerate on the next build.
set -euo pipefail

apply=0 incremental=0 include_main=0 older_than=0
while [ $# -gt 0 ]; do
  case $1 in
    --apply) apply=1 ;;
    --incremental) incremental=1 ;;
    --include-main) include_main=1 ;;
    --older-than) older_than=${2:?--older-than needs a number of days}; shift ;;
    -h|--help) sed -n '2,/^set /p' "$0" | sed '$d;s/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done
case $older_than in ''|*[!0-9]*) echo "--older-than expects whole days" >&2; exit 2 ;; esac

main=$(git rev-parse --show-toplevel)

busy() { # is a cargo/rustc process running inside $1?
  local pid cwd
  for pid in $(pgrep -x 'cargo|rustc' || true); do
    cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null || true)
    case $cwd in "$1"|"$1"/*) return 0 ;; esac
  done
  return 1
}

newest_mtime() { find "$1" -maxdepth 2 -printf '%T@\n' | sort -n | tail -1 | cut -d. -f1; }

total=0
while read -r wt; do
  [ "$wt" = "$main" ] && [ "$include_main" -eq 0 ] && continue
  [ -d "$wt/target" ] || continue

  if [ "$older_than" -gt 0 ]; then
    age=$(( ($(date +%s) - $(newest_mtime "$wt/target")) / 86400 ))
    [ "$age" -ge "$older_than" ] || { echo "skip (touched ${age}d ago)  $wt"; continue; }
  fi
  if busy "$wt"; then echo "skip (cargo running)  $wt"; continue; fi

  if [ "$incremental" -eq 1 ]; then
    paths=()
    for p in "$wt"/target/*/incremental; do [ -d "$p" ] && paths+=("$p"); done
    [ ${#paths[@]} -gt 0 ] || continue
  else
    paths=("$wt/target")
  fi

  kb=$(du -sk "${paths[@]}" | awk '{s+=$1} END {print s+0}')
  total=$((total + kb))
  printf '%8s  %s\n' "$(numfmt --to=iec --from-unit=1024 "$kb")" "$wt"
  [ "$apply" -eq 1 ] && rm -rf -- "${paths[@]}"
done < <(git worktree list --porcelain | sed -n 's/^worktree //p')

verb='would free'; [ "$apply" -eq 1 ] && verb='freed'
echo "$verb $(numfmt --to=iec --from-unit=1024 "$total")"
[ "$apply" -eq 1 ] || echo "dry run: pass --apply to delete"
