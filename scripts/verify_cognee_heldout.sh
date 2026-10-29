#!/bin/sh
# Preserve complete final gate logs and statuses. No private session text.
set -u
cd "$(dirname "$0")/.." || exit 1
umask 077
export CARGO_TARGET_DIR=target/cognee-remaining-validation
export CARGO_BUILD_JOBS=2
export RUSTFLAGS='-D warnings'
export RUSTDOCFLAGS='-D warnings'
export CARGO_NET_OFFLINE=true
export ICERS_OFFLINE=1
ICERS_CODING_AGENT_DIR=$(mktemp -d /tmp/icers-heldout-gates.XXXXXX) || exit 1
export ICERS_CODING_AGENT_DIR
logdir=$CARGO_TARGET_DIR/heldout-gates
mkdir -p "$logdir" || exit 1
: > "$logdir/statuses.tsv"
failed=0
run() {
    name=$1
    shift
    "$@" > "$logdir/$name.log" 2>&1
    code=$?
    printf '%s\t%s\n' "$name" "$code" >> "$logdir/statuses.tsv"
    printf '%s exit=%s\n' "$name" "$code"
    if [ "$code" -ne 0 ]; then failed=1; fi
}
run preparation-tests python3 -I scripts/test_prepare_cognee_heldout.py
run fmt cargo fmt --all -- --check
run check cargo check --workspace --all-targets --locked
run test cargo test --workspace --locked --no-fail-fast
run embedded sh -c 'sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs'
run prebuilt python3 -I scripts/test_prebuilt_release.py
run diff git diff --check
exit "$failed"
