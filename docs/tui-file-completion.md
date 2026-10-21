# File completion in the TUI

Type `@` to browse files, or follow it with a filename fragment such as
`@parser` to find matching files and directories throughout the working tree.
When `fd` (or `fdfind`) is available on `PATH`, recursive search runs in the
background and respects ignore files. Hidden files are included; `.git`
internals are excluded. The search follows symlinks and displays up to twenty
ranked results from a bounded search of one hundred entries.

Exact filenames rank first, followed by filename prefixes, filename substrings,
and path matches. Directory matches receive preference at the same match level.
Queries are case insensitive. A directory prefix such as `@src/par` searches
within that directory; a fragment such as `@tui/src/auto` can also match the
middle of a deeper path. While a recursive search runs, or when neither search
executable is installed, the menu still offers ordinary directory completion.

Use the arrow keys to select an entry and Tab to accept it. Completion replaces
the current path token and keeps the surrounding draft. Paths containing spaces
are quoted automatically. Completing a quoted directory leaves the caret inside
the closing quote so you can keep typing its child path. For example, completing
`@my` may produce `@"my folder/"`, then completing a child may produce
`@"my folder/test file.txt" `.

Directory listings filter matching entries before applying their fifty-result
limit, so matching files are discoverable even in large directories. Results
are ordered by directories first and then by name.