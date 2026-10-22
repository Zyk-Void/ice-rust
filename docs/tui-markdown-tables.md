# Markdown tables in the TUI

Table cells wrap onto additional lines when a table is wider than the terminal.
Long words, URLs and inline code remain complete, and styled text keeps its
formatting across wrapped lines. Each table row expands to the height of its
tallest cell, with borders separating adjacent rows.

Column allocation accounts for the longest word where space permits and never
assigns a Unicode grapheme less than its display width. Empty columns retain
their positions. When the terminal is too narrow for the column structure, the
original Markdown rows wrap as plain source so the answer's text remains
available. Horizontal padding is included in the width budget.

Regression fixtures sweep every terminal width from twenty through one hundred
sixty columns, including padded layouts, varying column widths, inline styles,
unbroken tokens, combining marks, wide characters, emoji and empty cells.