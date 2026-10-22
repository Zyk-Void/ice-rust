# Fork from a previous prompt

Run `/fork` to choose a user message from the saved session. The picker starts
on the latest message; use the arrow keys or type to filter the list, then press
Enter. Press Escape to return to your current draft without changing the session.
When `doubleEscapeAction` is set to `fork`, double Escape in an empty editor also
opens the fork picker.

Selecting a message creates a new session containing the conversation before that
message. The selected prompt text is placed in the editor, including its newlines,
so you can revise it before sending. Forking from the first prompt starts with an
empty conversation; forking from a later prompt retains its
earlier turns. The original session stays available with its full conversation.

The new session is saved beside the active session file, including sessions loaded
from a custom directory or another project. It receives a new session identifier,
records the original session as its parent, and owns a separate session lock.
Transcript panels, usage totals, and the text used by `/copy` follow the forked
history.

Forking is available while the agent is idle. A selection from a different active
file or a message no longer available is rejected; reopen `/fork` to
choose from the current conversation. Extension requests to fork the whole branch
use the same storage and locking behavior.

Regression coverage exercises first, earlier, and latest prompts; multiline
Unicode text and editor cursor placement; an active file sharing its identifier
with a different saved session; custom and cross-project storage; retained usage;
new-session locking; cancellation; and stale or busy selections. The tests compare
the original JSONL bytes before and after each fork.