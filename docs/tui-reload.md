# Refreshing commands with `/reload`

After changing loaded extensions or prompt templates, run `/reload` while the
agent is idle. A successful reload replaces the command registry and the `/`
completion provider together. Newly registered commands appear, removed commands
disappear, and changed descriptions are refreshed. Built-in commands and their
aliases keep precedence over extension commands with the same name.

The completion provider also refreshes prompt templates and retains file-path
completion rooted at the current working directory. A reload failure is shown
in the transcript; it does not install a partial command list.

To verify interactively, add a prompt template, reload, and check that its name
is offered after typing `/`. Remove or rename it and repeat. For an already
installed extension, repeat with a registered command and its description;
the old command must no longer dispatch after removal. No extension installation
is required by the automated tests.