#!/bin/sh
# Refresh the documentation snapshot that is compiled into the `ice` binary.
#
# crates/ice-cli/src/docs_tool.rs pulls these files in with include_str!, so they
# must be byte-copies of the dedicated docs/ice-tool/ sources. Keep these
# sources separate from docs/ ordinary user/developer documentation: the latter
# may contain release notes, website-oriented pages, or implementation notes
# that should not become part of the model-facing docs tool.
# The changelog is bundled separately for TUI startup notices and /changelog;
# it is not registered as a model-facing docs topic.
#
# CI runs this script and fails on any resulting diff, so drift is caught at the
# point it is introduced rather than at release time.

set -eu

DEST="crates/ice-cli/embedded-docs"

# source -> destination file name
FILES="docs/ice-tool/README.md:README.md \
docs/ice-tool/agent-project.md:agent-project.md \
docs/ice-tool/architecture.md:architecture.md \
docs/ice-tool/extension-authoring.md:extension-authoring.md \
docs/ice-tool/rust-debugging.md:rust-debugging.md \
docs/ice-tool/user-guide.md:user-guide.md \
CHANGELOG.md:changelog.md"

for entry in $FILES; do
  src="${entry%%:*}"
  name="${entry##*:}"
  [ -f "$src" ] || { echo "missing source document: $src" >&2; exit 1; }
  cp "$src" "${DEST}/${name}"
done

echo "synced ${DEST} from the repository documents"
