#!/usr/bin/env bash
# Mechanical rpi -> ice rebrand of the upstream pi-rust tree.
#
# Idempotent: safe to re-run after merging new upstream commits. The `rebrand`
# branch must contain nothing but the output of this script (see FORK.md), so
# that upstream syncs can always be resolved by re-running it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Files this script never rewrites: itself, fork docs.
SKIP='^(scripts/ice-rebrand\.sh|scripts/ice-sync-upstream\.sh|FORK\.md)$'
# History files: links to upstream PRs/issues/commits must keep pointing upstream.
HISTORY='^(CHANGELOG\.md|NOTICE|crates/(rpi|ice)-cli/embedded-docs/changelog\.md)$'

# Tokens that contain "rpi" but are not ours: external npm package, upstream domain.
protect='s/rpiv-todo/\x00K1\x00/g; s/rpi\.laofu/\x00K2\x00/g;'
restore='s/\x00K1\x00/rpiv-todo/g; s/\x00K2\x00/rpi.laofu/g;'
# Standalone `rpi` (the binary, ~/.rpi, the product name) becomes `icers` and
# env vars/consts become ICERS_*, so this build never touches TS ice's `ice`
# binary, ~/.ice dir or ICE_* env. Compound names (rpi-cli, rpi_ai) become ice-*.
names='s/CARGO_BIN_EXE_rpi\b/CARGO_BIN_EXE_icers/g; s/(?<![A-Za-z0-9_-])rpi(?![A-Za-z0-9_-])/icers/g; s/rpi/ice/g; s/RPI/ICERS/g; s/Rpi/Ice/g;'

# 1. Paths.
git ls-files | { grep -iE 'rpi' || true; } | { grep -vE "$SKIP" || true; } | while read -r f; do
  new=$(printf '%s' "$f" | perl -pe 's/bigfish1913\.rpi/Zyk-Void.ice/g; '"$names")
  [ "$f" = "$new" ] && continue
  mkdir -p "$(dirname "$new")"
  git mv "$f" "$new"
  rmdir -p "$(dirname "$f")" 2>/dev/null || true
done

# 2. Contents.
{ git grep -Il -e rpi -e RPI -e Rpi -e bigfish1913 -e pi-rust || true; } | { grep -vE "$SKIP" || true; } | while read -r f; do
  if [[ "$f" =~ $HISTORY ]]; then
    perl -pi -e "$protect"' s#github\.com/bigfish1913/pi-rust/(pull|issues|compare|commit)#\x00K3\x00$1#g; '"$names"' s#\x00K3\x00#github.com/bigfish1913/pi-rust/#g; '"$restore" "$f"
  else
    perl -pi -e "$protect"' s#bigfish1913/pi-rust#Zyk-Void/ice-rust#g; s/bigfish1913\.rpi/Zyk-Void.ice/g; s/\bpi-rust\b/ice-rust/g; '"$names $restore" "$f"
  fi
done
# Code compares repository URLs in canonical (lowercase) form, e.g.
# prebuilt_extensions::canonical_repository; docs keep the display casing.
{ git grep -Il 'Zyk-Void/ice-rust' -- '*.rs' || true; } | while read -r f; do
  perl -pi -e 's#Zyk-Void/ice-rust#zyk-void/ice-rust#g' "$f"
done
# Renamed packages sort differently; let cargo rewrite Cargo.lock in its own order.
cargo metadata --format-version 1 --offline >/dev/null 2>&1 || cargo metadata --format-version 1 >/dev/null
# Renames change line lengths, and CI checks rustfmt and the embedded-docs copy.
cargo fmt --all
sh scripts/sync-embedded-docs.sh >/dev/null
git add -u

# 3. Anything left is unexpected: fail so a human looks at it.
left=$(git grep -IhoE '[A-Za-z0-9_./@-]*(rpi|RPI|Rpi)[A-Za-z0-9_./-]*' -- . \
  ':!scripts/ice-rebrand.sh' ':!scripts/ice-sync-upstream.sh' ':!FORK.md' \
  ':!CHANGELOG.md' ':!NOTICE' ':!crates/ice-cli/embedded-docs/changelog.md' \
  | grep -vE 'rpiv-todo|rpi\.laofu' | sort -u || true)
if [ -n "$left" ]; then
  printf 'ice-rebrand: unexpected leftovers:\n%s\n' "$left" >&2
  exit 1
fi
echo "ice-rebrand: done"
