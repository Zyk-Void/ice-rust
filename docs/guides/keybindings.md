# Keyboard shortcuts

`/hotkeys` lists the effective bindings used by the running TUI, including
unbound actions. Each row includes its action ID, which can be copied into the
configuration file. Editing the file takes effect on the next launch.

Create `~/.icers/agent/keybindings.json` (or `keybindings.json` in the directory
selected by `ICERS_CODING_AGENT_DIR`):

```json
{
  "app.model.select": "alt+m",
  "app.model.cycleBackward": ["ctrl+shift+p", "alt+p"],
  "tui.editor.cursorWordLeft": "alt+h",
  "tui.select.confirm": "alt+enter",
  "tui.editor.undo": []
}
```

A string sets one shortcut; an array sets alternatives. An empty array unbinds
the action. Names are case-insensitive for key notation, such as `ctrl+shift+p`,
`escape`, `f6`, `space`, and `alt+enter`. Function keys F1–F24 and the literal
plus key (`ctrl++`) are supported. Shift+Tab is normalized across terminals that
report it as either BackTab or Tab with Shift.

Precedence is built-in defaults, then the older `settings.json#keybindings`
object, then `keybindings.json`. The file can override every registered action,
including editor movement and selector confirmation, rather than only a small
list of app shortcuts. Selectors use their own contextual actions; a shortcut
may therefore have different meanings in an editor and a selector.

Legacy names such as `cursorUp`, `expandTools`, and `selectConfirm` remain
readable. If both legacy and namespaced forms exist in one source, the
namespaced form wins. Migration happens in memory: the loader does not rewrite
or delete entries in the user's file. Unknown actions and invalid entries are
ignored. A malformed array is rejected as a whole, so one invalid key cannot
silently remove a working shortcut. A malformed or unreadable file leaves
settings overrides and defaults available.

Some terminal emulators intercept their own shortcuts before the application
receives them. Choose keys that your terminal forwards. Deferred features do
not acquire actions just because their names appear in a copied configuration.
