//! Mirrors `packages/coding-agent/src/core/tools/index.ts` — named built-in
//! tools and the standard tool sets SDK users build agents from.
//!
//! The seven names are ice's public built-in set. Background-shell controls
//! (`bash_output`, `bash_kill`, `bash_input`, `bash_watch`) are deliberately
//! not in it: a host creates them with the session's registry when bash is
//! active. The Rust-only PowerShell tool is likewise outside the set.
//!
//! write and edit need a mutation-capable context; asking for them without
//! one is a construction error rather than a panic.

use std::sync::Arc;

use ice_agent::agent_tool::AgentTool;

use crate::tools::bash::{create_bash_tool, BashToolOptions};
use crate::tools::edit::create_edit_tool;
use crate::tools::find::{create_find_tool, FindToolOptions};
use crate::tools::grep::{create_grep_tool, GrepToolOptions};
use crate::tools::ls::{create_ls_tool, LsToolOptions};
use crate::tools::read::{create_read_tool, ReadToolOptions};
use crate::tools::tool_context::ExecutionToolContext;
use crate::tools::write::create_write_tool;

/// A built-in tool name. Mirrors TS `ToolName`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    Read,
    Bash,
    Edit,
    Write,
    Grep,
    Find,
    Ls,
}

impl ToolName {
    /// Every built-in, in ice's order. Mirrors `allToolNames`.
    pub const ALL: [ToolName; 7] = [
        ToolName::Read,
        ToolName::Bash,
        ToolName::Edit,
        ToolName::Write,
        ToolName::Grep,
        ToolName::Find,
        ToolName::Ls,
    ];
    /// The coding set. Mirrors `createCodingTools`.
    pub const CODING: [ToolName; 4] = [
        ToolName::Read,
        ToolName::Bash,
        ToolName::Edit,
        ToolName::Write,
    ];
    /// The read-only set. Mirrors `createReadOnlyTools`.
    pub const READ_ONLY: [ToolName; 4] =
        [ToolName::Read, ToolName::Grep, ToolName::Find, ToolName::Ls];

    pub fn as_str(self) -> &'static str {
        match self {
            ToolName::Read => "read",
            ToolName::Bash => "bash",
            ToolName::Edit => "edit",
            ToolName::Write => "write",
            ToolName::Grep => "grep",
            ToolName::Find => "find",
            ToolName::Ls => "ls",
        }
    }
}

impl std::str::FromStr for ToolName {
    type Err = String;

    /// Mirrors the factory switch's `Unknown tool name: <name>` error.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        ToolName::ALL
            .into_iter()
            .find(|t| t.as_str() == name)
            .ok_or_else(|| format!("Unknown tool name: {name}"))
    }
}

/// Per-tool options; each tool receives only its own. Mirrors `ToolsOptions`.
#[derive(Clone, Default)]
pub struct ToolsOptions {
    pub read: Option<ReadToolOptions>,
    pub bash: Option<BashToolOptions>,
    pub grep: Option<GrepToolOptions>,
    pub find: Option<FindToolOptions>,
    pub ls: Option<LsToolOptions>,
}

/// Build one built-in tool. Mirrors `createTool`.
pub fn create_tool(
    name: ToolName,
    context: &ExecutionToolContext,
    options: &ToolsOptions,
) -> Result<Arc<dyn AgentTool>, String> {
    let needs_mutation = matches!(name, ToolName::Edit | ToolName::Write);
    if needs_mutation && context.mutating_env().is_none() {
        return Err(format!(
            "The {} tool needs a mutation-capable execution context.",
            name.as_str()
        ));
    }
    Ok(match name {
        ToolName::Read => create_read_tool(context, options.read.clone()),
        ToolName::Bash => create_bash_tool(context, options.bash.clone()),
        ToolName::Edit => create_edit_tool(context),
        ToolName::Write => create_write_tool(context),
        ToolName::Grep => create_grep_tool(context, options.grep.clone()),
        ToolName::Find => create_find_tool(context, options.find.clone()),
        ToolName::Ls => create_ls_tool(context, options.ls.clone()),
    })
}

/// Build the named tools in order.
pub fn create_tools(
    names: &[ToolName],
    context: &ExecutionToolContext,
    options: &ToolsOptions,
) -> Result<Vec<Arc<dyn AgentTool>>, String> {
    names
        .iter()
        .map(|name| create_tool(*name, context, options))
        .collect()
}

/// `[read, bash, edit, write]`. Mirrors `createCodingTools`.
pub fn create_coding_tools(
    context: &ExecutionToolContext,
    options: &ToolsOptions,
) -> Result<Vec<Arc<dyn AgentTool>>, String> {
    create_tools(&ToolName::CODING, context, options)
}

/// `[read, grep, find, ls]`; needs no mutation capability. Mirrors
/// `createReadOnlyTools`.
pub fn create_read_only_tools(
    context: &ExecutionToolContext,
    options: &ToolsOptions,
) -> Vec<Arc<dyn AgentTool>> {
    create_tools(&ToolName::READ_ONLY, context, options)
        .expect("read-only tools need no mutation capability")
}

/// Every built-in by name, in ice's order. Mirrors `createAllTools`.
pub fn create_all_tools(
    context: &ExecutionToolContext,
    options: &ToolsOptions,
) -> Result<Vec<(ToolName, Arc<dyn AgentTool>)>, String> {
    ToolName::ALL
        .into_iter()
        .map(|name| create_tool(name, context, options).map(|tool| (name, tool)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contexts() -> (ExecutionToolContext, ExecutionToolContext) {
        let env = Arc::new(crate::InMemoryExecutionEnv::with_cwd("/w".into()));
        let env_dyn: Arc<dyn crate::ExecutionEnv> = env.clone();
        let mutating: Arc<dyn crate::MutatingEnv> = env;
        (
            ExecutionToolContext::new(env_dyn.clone(), Some(mutating)),
            ExecutionToolContext::new(env_dyn, None),
        )
    }

    fn names(tools: &[Arc<dyn AgentTool>]) -> Vec<String> {
        tools.iter().map(|t| t.schema().name.clone()).collect()
    }

    #[test]
    fn standard_sets_keep_ice_order() {
        let (full, read_only) = contexts();
        let options = ToolsOptions::default();
        assert_eq!(
            names(&create_coding_tools(&full, &options).unwrap()),
            ["read", "bash", "edit", "write"]
        );
        assert_eq!(
            names(&create_read_only_tools(&read_only, &options)),
            ["read", "grep", "find", "ls"]
        );
        let all: Vec<&str> = create_all_tools(&full, &options)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(all, ["read", "bash", "edit", "write", "grep", "find", "ls"]);
    }

    #[test]
    fn mutation_tools_need_a_mutating_context_and_names_are_checked() {
        let (_, read_only) = contexts();
        let err = create_coding_tools(&read_only, &ToolsOptions::default())
            .err()
            .expect("edit needs mutation");
        assert_eq!(
            err,
            "The edit tool needs a mutation-capable execution context."
        );
        assert_eq!("grep".parse::<ToolName>(), Ok(ToolName::Grep));
        assert_eq!(
            "bash_output".parse::<ToolName>(),
            Err("Unknown tool name: bash_output".to_string())
        );
    }
}
