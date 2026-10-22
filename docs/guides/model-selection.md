# Selecting a model

Use `/model` to open the model picker. Favorites (toggle with `shift+f`) are pinned
at the top and the picker opens there; without favorites it opens on the
currently active model. Type to
filter by model name, provider, or ID; a new query selects its first match.
`/model provider/model` selects an exact model directly. A query without an
exact match opens the picker with that query filled in.

The picker closes after a choice, and the switch runs in the background.
The footer and context-window badge continue to show the previous active model
until the lane accepts the new one. A successful switch then updates the footer
and reports that it applies to the next message. A run already in progress
continues using its existing model. A rejected switch reports the error and
leaves the active model and saved default unchanged.

A model chosen in `/model` or in the default-model picker in `/settings` is
saved as the default for the next launch, whichever provider it belongs to.
Saving happens after the live switch succeeds. If saving fails, the UI reports
that the model is active but the default was not saved.
Model cycling updates its footer only after acceptance and reports failures; it
does not change the saved default.

These changes use the existing `defaultProvider` and `defaultModel` settings;
they add no on-disk fields or session-record format changes.
