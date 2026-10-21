# Renaming and deleting saved sessions

In `/session`, F2 opens the selected session's name editor. Enter saves the name;
Esc discards the edit and returns to the same picker. A blank name clears the
saved name and restores its conversation preview. Names are appended as ordinary
session facts, so existing transcript bytes are preserved.

Ctrl+D opens a deletion confirmation showing the selected session and its path.
Enter deletes that saved JSONL file; Esc leaves it unchanged. The active session
cannot be deleted. A session still held open by an icers process cannot be edited
through another storage handle. Restart the owning process when you need to
release an old session that it previously opened.

Rename, delete, and scope are registered as `app.session.rename`,
`app.session.delete`, and `app.session.scope`. The picker displays their effective
bindings. File work runs in the background; after a confirmed operation finishes,
the picker refreshes its rows and reports the result. A changed file must be
reselected in a reopened picker before editing. Damaged logs must be recovered
through the normal resume flow before renaming; a failed rename does not repair
or rewrite their history.

Manual check: in disposable project fixtures, create two sessions with equal IDs
in different directories. Rename and delete one, checking the other remains
unchanged. Cancel both prompts, try deleting the active session, and try editing
a session held open by a second process. Keep the picker open after a failed
operation and check that the error is visible and the selected file still exists.
