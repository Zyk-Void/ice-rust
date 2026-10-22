# Fullscreen selection copy

In fullscreen mode, drag with the left mouse button to copy the visible screen,
including the prompt, status and footer dock, borders, and visible overlay text.
The selection uses the displayed screen frame, so text hidden under an overlay
is not included. Graphics remain terminal graphics and are not copied as
images. Copied text has no terminal styling codes or trailing padding. A
prompt click places the caret; a click without a drag does not copy.

Drags that stay in the primary transcript keep their starting point attached to
its logical row if you scroll while holding the button. Drags that begin in the
dock or cross into another screen region use the captured visible frame until
release, so live output cannot move the text under the pointer. Scrolling during
such a screen drag cancels it. A terminal resize, keyboard editing, or overlay
open/close also cancels a selection safely. Whole Unicode graphemes are copied
at the selection's right edge, including combining marks, CJK characters, and
joined emoji.

For a physical-terminal check on Linux, use a disposable ICERS configuration
directory, open a long transcript at a width above eighty columns, scroll up,
and drag across two partial lines. Paste into another application and compare
the first and last selected words. Repeat with styled text, wide Unicode text,
and scrolling during a drag. These scenarios complement the automated layout
and extraction tests.
