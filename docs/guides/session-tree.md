# Session tree

`/tree` shows saved entries in their parent/child hierarchy. Branch connectors
separate alternatives, and a dot marks entries on the current branch. The
current entry is marked explicitly; its visible ancestor is selected when the
current entry is hidden by a filter.

Each row previews user and assistant text, tool results, model and thinking
changes, compactions, branch summaries, or custom messages. Sequence numbers and
short IDs distinguish similar rows. Search uses full content, entry type, ID,
sequence, and labels, including words beyond the preview.

## Navigation

The default keys are:

| Key | Action |
| --- | --- |
| Up / Down | Move through visible entries |
| Page Up / Page Down, Left / Right | Move one page |
| Ctrl+Left / Alt+Left | Fold a branch, or move to its parent segment |
| Ctrl+Right / Alt+Right | Unfold a branch, or move to its child segment |
| Enter | Navigate to the selected entry |
| Escape | Clear the search first; close the picker when search is empty |
| Tab / Shift+Tab | Cycle the entry filter |
| Ctrl+D | Default entries |
| Ctrl+T | Hide tool results |
| Ctrl+U | User messages only |
| Ctrl+L | Labeled entries only |
| Ctrl+A | All entries, including model/thinking/tool metadata |

Filters keep the closest visible ancestor selected. An empty result retains
the remembered selection so returning to the previous filter restores it.
Search and filter changes clear folds, ensuring matching descendants can be
found. Folding a parent preserves any folds inside it when reopened.

These controls are registered keybinding actions. The picker displays the
effective keys after overrides. Previews fit within 80 terminal columns without
splitting wide characters or emoji; deep trees pan horizontally to keep the
selected text visible. Long linear conversation segments stay compact.

Selecting a row moves within the current session. Closing the picker changes
neither the session position nor its saved entries.
