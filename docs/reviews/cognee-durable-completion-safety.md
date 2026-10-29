# Cognee durable completion safety review

Scope: bounded review of admission receipts, queue recovery, accepted candidate projection, and drain completion handling.

- Receipt scans inspect at most 4,096 entries; only regular, non-symlink JSON files up to 64 KiB are considered. Invalid entries are ignored (diagnostic/quarantine only; no guessed repair or deletion).
- Receipt identity and nested candidate/operation admission IDs, candidate UUID, dataset/node-set/category relationship, accepted/prepared states, operation checksum/ID, source-scope hash, and superseded-target scope/hash are validated before a receipt is trusted. Existing receipt IDs are idempotent only for byte-identical serialized payloads.
- Completion markers require a valid operation ID and regular bounded JSON containing exactly `{"completed":true}`. Empty/forged markers do not suppress recovery.
- Reconciliation takes dispatch ownership before the admission lock and local queue operations; it performs no HTTP. Newly restored prepared work is activated in the same pass. Existing in-flight/uncertain state is retained, and committed completion markers prevent queue recreation.
- An admitted wire success whose completion-marker write fails remains uncertain, never automatically pending. The candidate recall metadata retains source scope in snake_case.

Validation run on the current branch after `cargo clean -p ice-cli`:

- `cargo test -p ice-cli cognee::admission --lib` — 7 passed
- `cargo test -p ice-cli cognee::queue --lib` — 8 passed
- `cargo test -p ice-cli cognee::candidates --lib` — 5 passed
- `cargo test -p ice-cli cognee:: --lib` — 182 passed
