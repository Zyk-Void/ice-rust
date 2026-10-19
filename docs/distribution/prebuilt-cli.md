# Prebuilt CLI distribution protocol v1

`ice update` resolves the released `ice-cli` version through crates.io and
downloads its prebuilt executable from the official GitHub repository. Cargo
compilation is an explicit alternative: `ice update --build`.

This uses the same envelope and integrity rules as the extension package
protocol. CLI Releases retain their existing `v<version>` tags. They publish
`ice-release.json`, and each target declares `binary` instead of `library`.
An executable is self-contained, so it does not require the running host's
plugin SDK ABI to match. The updater validates its exact `ice --version` output.

```json
{
  "schema_version": 1,
  "name": "ice-cli",
  "version": "0.3.24",
  "repository": "https://github.com/Zyk-Void/ice-rust",
  "tag": "v0.3.24",
  "source_commit": "0123456789abcdef0123456789abcdef01234567",
  "targets": {
    "x86_64-pc-windows-msvc": {
      "url": "https://github.com/Zyk-Void/ice-rust/releases/download/v0.3.24/ice-v0.3.24-x86_64-pc-windows-msvc.zip",
      "format": "zip",
      "size": 123456,
      "sha256": "<64 lowercase hexadecimal characters>",
      "binary": {
        "path": "ice.exe",
        "sha256": "<64 lowercase hexadecimal characters>"
      }
    }
  }
}
```

The example abbreviates the target map. Official releases contain Windows x64
MSVC, Linux x64/ARM64 GNU, and macOS Intel/Apple Silicon. No architecture or
GNU/musl substitution is allowed. Download URLs must identify the same official
repository and versioned Release. Unknown fields may be ignored; unknown schema
versions are rejected.

The updater verifies the compressed archive's byte length and SHA-256, extracts
only the declared flat `ice`/`ice.exe` file, and verifies its SHA-256 and native
object architecture. Archive traversal, links, duplicate executable entries and
oversized payloads are rejected using the extension installer's shared checks.
The downloaded executable must report exactly the requested version and must
not downgrade the running CLI.

Both download and explicit source-build paths feed the existing private staging
and atomic replacement transaction. On Windows a hidden helper waits for the
current process to exit, validates the replacement again, and rolls back on
failure. On Unix the replacement uses the same directory as the current
executable and rolls back if post-replacement validation fails.

`scripts/prebuilt-release.py` records each target's archive and executable hashes
and source commit. The release job verifies all five artifacts refer to the same
version and commit, preserves the existing archive names and `.sha256` files,
uploads archives first, and uploads `ice-release.json` last. The release driver
waits for this manifest before declaring the binaries ready.

A missing manifest means the prebuilt release is not ready. This includes older
Releases created before this protocol. The command reports the error without
falling back to source compilation or replacing the installed executable. Retry
after the assets are published, or explicitly use `ice update --build`.

`--force` permits reinstalling the resolved version; `--offline` only reports
cached update information and performs no downloads or builds.
