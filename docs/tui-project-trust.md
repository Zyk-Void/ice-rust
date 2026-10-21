# Project trust in the TUI

`/trust` displays the saved trust decision for the current project and the
effective trust state of this running session. It does not change either state.
These can differ after a saved decision changes or when startup flags override
the saved choice.

Use `/trust yes` to save trust, `/trust no` to save a denial, or `/trust clear`
to remove the saved decision. Restart `icers` to apply the changed decision to
project resources. Clearing a saved decision does not immediately change which
resources are loaded in the current session.