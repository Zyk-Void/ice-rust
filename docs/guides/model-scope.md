# Session model scope

Use `/scoped-models` to choose and order the models offered by model cycling.
Type to search by provider, model ID, or friendly name. Arrow keys move through
matches; Space toggles the highlighted model. Alt+Up and Alt+Down reorder
selected models without losing the highlighted model while a search is active.

- **Enter** applies the working selection to the current running session only.
- **Ctrl+S** saves it to `settings.json` and applies it to the current session.
- **Escape** discards changes made since opening the selector.
- **Ctrl+A** enables matching models, or restores all models when search is empty.
- **Ctrl+X** clears matching models; **Ctrl+P** toggles the highlighted provider.
- **Ctrl+C** clears an active search, or cancels when the search is empty.

These shortcuts are listed in the selector and respect configured bindings.
The selector uses a working copy: it does not change the active cycle or write
settings until Enter or Ctrl+S. A failed save keeps the selector open and retains
the working selection so it can be retried; the active selection is unchanged.
This explicit cancel behavior follows the TUI goal and differs deliberately from
the TypeScript selector's immediate session updates.

The saved order is also the cycle order. IDs are saved as `provider/model`, while
legacy bare IDs remain readable. Unavailable configured models stay visible and
are retained in the saved list; cycling skips them. Cycling remains within the
active provider because the live lane has that existing routing constraint.
Use `/model` or `/provider` to choose outside the cycle.

An absent or empty saved scope retains the existing meaning: all models. When a
working selection is empty, the selector explicitly says that applying it
restores all models. Session-only changes last for this running TUI; restarting
uses the last explicitly saved selection. No credentials, session JSONL records,
or TypeScript configuration files are changed by this command.
