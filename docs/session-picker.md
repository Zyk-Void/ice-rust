# Saved sessions

Open `/session` (or `/resume`) to find a saved conversation. The picker starts in the current folder; press **Tab** to switch between the current folder and all indexed projects. Typing filters names, IDs, and project paths. Enter resumes the highlighted conversation; Escape returns to the draft. The query is preserved when changing scope.

The index remembers session directories visited by interactive `icers`, including an explicit `--session-dir` or saved `sessionDir`. Existing projects become discoverable after launching `icers` there once. A legacy global `<agent directory>/sessions` directory is also included. The picker does not scan the whole home directory. Session loading runs away from the input thread.

Each index entry is an independent atomic JSON file under `<agent directory>/session-directories/`, keyed by a SHA-256 of the canonical directory. The file contains only a directory path. This adds discoverability without moving or rewriting existing JSONL files. Missing directories and malformed index entries are ignored; duplicate directories and file paths are deduplicated. Equal session IDs in different projects remain separate because selection uses the exact absolute file path.

Resuming another project's conversation retains the current execution directory and current runtime configuration. The original project path is shown in the picker so the distinction is visible before selection.

## Verification recipe

Use two disposable projects and an isolated `ICERS_CODING_AGENT_DIR`, make a saved conversation in each, then open `/session` in one. Verify Tab includes both, a query survives the scope toggle, Enter opens the selected file, and Escape preserves the prompt draft. No live provider is required for automated coverage.
