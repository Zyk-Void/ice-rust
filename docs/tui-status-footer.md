# Git branches in the status footer

The status footer shows the current Git branch beside the working directory.
Branch names keep their full slash-separated path: for example,
`feature/status-footer` appears as `(feature/status-footer)`.

The footer finds the repository enclosing the working directory, including when
you start `icers` from a subdirectory. In a linked worktree, it follows the
worktree's `.git` file to its own Git metadata and reads that worktree's `HEAD`.
Both relative and absolute `gitdir` paths are supported, so the branch belongs
to the linked worktree you are using. Branch changes refresh through a watcher
on the resolved `HEAD` file.

For a detached checkout, the branch position shows the first seven characters
of the commit ID. Outside a Git repository, or when its `HEAD` cannot be read,
the footer omits the branch label.
