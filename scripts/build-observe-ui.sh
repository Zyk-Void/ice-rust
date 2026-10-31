#!/bin/sh
# Build the observe dashboard (web/observe-ui, Angular + spartan/ui) and copy
# the bundle that crates/ice-cli embeds into crates/ice-cli/src/observe/ui/.
# Run it after changing anything under web/observe-ui, and commit the output.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
app="$root/web/observe-ui"
out="$root/crates/ice-cli/src/observe/ui"

cd "$app"
npm ci
npx ng build --configuration production

# The server embeds exactly these files; anything else (a lazy chunk, a font
# file) would 404 at runtime.
expected="3rdpartylicenses.txt index.html main.js styles.css"
actual=$(ls dist | sort | tr '\n' ' ' | sed 's/ $//')
if [ "$actual" != "$expected" ]; then
	echo "build-observe-ui: unexpected build output: $actual" >&2
	echo "build-observe-ui: expected: $expected (keep routes eager, assets inline)" >&2
	exit 1
fi

mkdir -p "$out"
cp dist/index.html dist/main.js dist/styles.css "$out/"
cp dist/3rdpartylicenses.txt "$out/licenses.txt"
echo "build-observe-ui: wrote $out"
