# Searching saved conversations

The in-TUI session picker searches session names, IDs, project paths, and saved
user, assistant, and tool-result text. Results retain their most-recent-first
order. Each ordinary whitespace-separated term must match; terms support fuzzy
character matching. Put a phrase in double quotes for a contiguous match (line
breaks count as spaces), or start the query with `re:` for a case-insensitive
regular expression. An invalid pattern shows an error and no selection.

Searching and file reads run on one background worker per open picker. A new
query clears the old selection immediately; obsolete searches cannot replace
new results. Esc remains available while loading or searching. Parsed text is
cached up to 16 MiB and invalidated when a file's size or modification time
changes. Larger logs are still searched in full without being retained in the
cache. Images and non-message metadata are excluded from content matching.
Unreadable files are counted visibly and can still match their row metadata.

For a manual check, open `/session` in an isolated agent/project fixture, search
for text present only in a late assistant answer, then try a quoted phrase and
`re:\banswer\b`. Repeat the search to exercise the cache, append another message,
and verify that it appears. Change queries rapidly in a large history and press
Esc during a scan; typing and cancellation should remain responsive.
