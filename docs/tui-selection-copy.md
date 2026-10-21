# Transcript selection copy

In fullscreen mode, drag with the left mouse button to copy transcript text.
The selection uses the transcript's actual wrapping width and scroll position.
The first and last lines include only the selected columns; intermediate lines
are copied in full. Copied text has no terminal styling codes or trailing
padding. The header, input dock, and reserved scrollbar column are outside the
transcript selection area.

The drag's starting point stays attached to its logical transcript row if you
scroll while holding the button. A terminal resize cancels the selection because
wrapping can change the meaning of row coordinates. A click without a drag does
not replace the clipboard. Whole Unicode graphemes are copied at the selection's
right edge, including combining marks and joined emoji.

For a physical-terminal check on Linux, use a disposable ICERS configuration
directory, open a long transcript at a width above eighty columns, scroll up,
and drag across two partial lines. Paste into another application and compare
the first and last selected words. Repeat with styled text, wide Unicode text,
and scrolling during a drag. These scenarios complement the automated layout
and extraction tests.