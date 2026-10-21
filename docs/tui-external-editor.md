# Editing a prompt in an external editor

Press **Ctrl+G** to edit the current draft. `icers` pauses its terminal renderer
and keyboard reader until the editor exits, including while a model response
is streaming. It restores the terminal and places the cursor at the end of the
saved draft when editing succeeds.

The command comes from `ICERS_EXTERNAL_EDITOR`, then `VISUAL`, then `EDITOR`.
The default is `nano` on Unix and `notepad` on Windows. Commands may include
whitespace-separated arguments such as `code --wait`; quoted executable paths
and shell pipelines are not parsed on Unix.

The draft is stored in a private temporary directory that is removed after
editing. A successful save replaces the prompt, including an empty save. One
final newline is removed. A nonzero exit cancels editing and keeps the original
draft; a failure to launch or read the saved file reports an error.

## Terminal verification

Run with a disposable `ICERS_CODING_AGENT_DIR`. Try `nano`, `vim`, and
`code --wait`; check multiline Unicode text, an empty save, and a cancelled
edit. Repeat while an offline faux-provider response streams. The `icers`
screen must stay quiet while the editor owns the terminal, and resume after it
exits. These physical-terminal checks supplement the automated fixture tests.