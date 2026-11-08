//! Instructions for the files the model touches.
//!
//! Startup loads the instruction files from the filesystem root down to the
//! working directory. A subdirectory's `AGENTS.md`, its `.icers/rules`, and
//! rules scoped with `paths:` apply only to some files, so they load when the
//! model first reads or changes one of those files. They are appended to that
//! tool result inside `<system-reminder>`, as Claude Code does for nested
//! `CLAUDE.md` files.
//!
//! Nothing is tracked between calls: a file is added again whenever its
//! `<project_instructions path="...">` element is no longer in the system
//! prompt or the conversation, for example after compaction summarized the
//! turn that carried it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ice_agent::hooks::AfterToolCall;
use ice_agent::types::AgentContext;
use ice_agent::{AgentMessage, TextContentOrImage};
use ice_ai::types::{Content, UserContent};
use ice_harness::context_files::{
    format_instruction_entries, instruction_marker, load_context_files_for_path, ContextFile,
};
use ice_tools::env::ExecutionEnv;

/// Tools whose `path` argument names a file the model is working on.
const PATH_TOOLS: &[&str] = &["read", "edit", "write"];

/// What the hook needs to find instruction files for a path.
pub struct NestedInstructions {
    pub env: Arc<dyn ExecutionEnv>,
    pub cwd: PathBuf,
    pub agent_dir: Option<PathBuf>,
    pub project_trusted: bool,
}

/// Wrap `previous` so a successful `read`/`edit`/`write` result also carries
/// the instruction files for its path that the model does not have yet.
pub fn hook(previous: Option<AfterToolCall>, config: Arc<NestedInstructions>) -> AfterToolCall {
    Arc::new(move |ctx, signal| {
        let target = (PATH_TOOLS.contains(&ctx.tool_call.name.as_str()) && !ctx.is_error)
            .then(|| ctx.args.get("path").and_then(|path| path.as_str()))
            .flatten()
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from);
        let present = target.as_ref().map(|_| present_markers(ctx.context));
        let original = ctx.result.content.clone();
        let previous_future = previous
            .as_ref()
            .map(|previous| previous(ctx, signal.clone()));
        let config = config.clone();
        Box::pin(async move {
            let base = match previous_future {
                Some(future) => future.await,
                None => None,
            };
            let (Some(target), Some(present)) = (target, present) else {
                return base;
            };
            let files: Vec<ContextFile> = load_context_files_for_path(
                &config.env,
                &config.cwd,
                config.agent_dir.as_deref(),
                &target,
                config.project_trusted,
            )
            .await
            .into_iter()
            .filter(|file| !present.contains(&instruction_marker(&file.path)))
            .collect();
            if files.is_empty() {
                return base;
            }
            let mut result = base.unwrap_or_default();
            result
                .content
                .get_or_insert(original)
                .push(TextContentOrImage::text(format_reminder(&target, &files)));
            Some(result)
        })
    })
}

fn format_reminder(target: &Path, files: &[ContextFile]) -> String {
    format!(
        "<system-reminder>\nInstructions that apply to {} follow. They were not loaded at \
startup; follow them for work on these files.\n\n{}</system-reminder>",
        target.display(),
        format_instruction_entries(files)
    )
}

/// Every `<project_instructions path="...">` opening visible to the model.
fn present_markers(context: &AgentContext) -> HashSet<String> {
    let mut markers = HashSet::new();
    let mut scan = |text: &str| {
        const OPEN: &str = "<project_instructions path=\"";
        let mut rest = text;
        while let Some(start) = rest.find(OPEN) {
            let after = &rest[start + OPEN.len()..];
            let Some(end) = after.find('"') else {
                break;
            };
            markers.insert(format!("{OPEN}{}\"", &after[..end]));
            rest = &after[end..];
        }
    };
    scan(&context.system_prompt);
    for message in &context.messages {
        match message {
            AgentMessage::User(user) => match &user.content {
                UserContent::Text(text) => scan(text),
                UserContent::Blocks(blocks) => blocks.iter().for_each(|block| {
                    if let Content::Text(text) = block {
                        scan(&text.text);
                    }
                }),
            },
            AgentMessage::ToolResult(result) => result.content.iter().for_each(|block| {
                if let Content::Text(text) = block {
                    scan(&text.text);
                }
            }),
            AgentMessage::Assistant(_) | AgentMessage::Custom(_) => {}
        }
    }
    markers
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_agent::types::{AfterToolCallContext, AgentToolResult};
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::agent_harness::{AgentHarness, AgentLane};
    use ice_harness::types::{AgentHarnessOptions, HarnessTool};
    use ice_tools::FileSystem;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    const WEB_MARKER: &str = "<project_instructions path=\"/proj/web/AGENTS.md\"";

    async fn project() -> Arc<ice_tools::InMemoryExecutionEnv> {
        let env = Arc::new(ice_tools::InMemoryExecutionEnv::with_cwd(PathBuf::from(
            "/proj",
        )));
        env.create_dir("/proj/web", true, None).await.unwrap();
        for (path, content) in [
            ("/proj/AGENTS.md", "ROOT_RULES"),
            ("/proj/web/AGENTS.md", "WEB_RULES"),
            ("/proj/web/a.txt", "a"),
            ("/proj/web/b.txt", "b"),
        ] {
            env.seed_file(path, content.as_bytes().to_vec()).await;
        }
        env
    }

    fn config(env: Arc<ice_tools::InMemoryExecutionEnv>, trusted: bool) -> Arc<NestedInstructions> {
        Arc::new(NestedInstructions {
            env,
            cwd: PathBuf::from("/proj"),
            agent_dir: None,
            project_trusted: trusted,
        })
    }

    /// Run the hook once for `tool` on `path` against `context`, returning the
    /// text it appended (if any).
    async fn appended(
        hook: &AfterToolCall,
        tool: &str,
        path: &str,
        is_error: bool,
        context: &AgentContext,
    ) -> Option<String> {
        let assistant =
            ice_ai::providers::faux::faux_assistant_message("", ice_ai::types::StopReason::ToolUse);
        let call = ice_ai::types::ToolCall {
            kind: ice_ai::types::ToolCallType,
            id: "call".into(),
            name: tool.into(),
            arguments: json!({ "path": path }),
            thought_signature: None,
            namespace: None,
        };
        let result = AgentToolResult {
            content: vec![TextContentOrImage::text("file body")],
            details: json!({}),
            usage: None,
            added_tool_names: Vec::new(),
            terminate: false,
        };
        let out = hook(
            AfterToolCallContext {
                assistant_message: &assistant,
                tool_call: &call,
                args: &call.arguments,
                result: &result,
                is_error,
                context,
            },
            CancellationToken::new(),
        )
        .await?;
        let content = out.content?;
        assert!(matches!(&content[0], TextContentOrImage::Text(t) if t.text == "file body"));
        content.iter().skip(1).find_map(|block| match block {
            TextContentOrImage::Text(text) => Some(text.text.clone()),
            TextContentOrImage::Image(_) => None,
        })
    }

    #[tokio::test]
    async fn adds_missing_instructions_and_restores_them_after_compaction() {
        let hook = hook(None, config(project().await, true));
        let mut context = AgentContext::default();
        let added = appended(&hook, "read", "web/a.txt", false, &context)
            .await
            .expect("instructions added");
        assert!(added.starts_with("<system-reminder>"));
        assert!(added.contains(WEB_MARKER) && added.contains("WEB_RULES"));
        assert!(
            !added.contains("ROOT_RULES"),
            "startup files are not repeated"
        );

        // Once the conversation carries the file, nothing is added again.
        context
            .messages
            .push(AgentMessage::User(ice_ai::types::UserMessage::new(
                added.clone(),
                0,
            )));
        assert!(appended(&hook, "edit", "/proj/web/b.txt", false, &context)
            .await
            .is_none());
        // After compaction drops that message, the next touch restores it.
        context.messages.clear();
        assert!(appended(&hook, "write", "web/b.txt", false, &context)
            .await
            .is_some_and(|text| text.contains("WEB_RULES")));
    }

    #[tokio::test]
    async fn ignores_other_tools_errors_and_untrusted_projects() {
        let env = project().await;
        let context = AgentContext::default();
        let trusted = hook(None, config(env.clone(), true));
        assert!(appended(&trusted, "bash", "web/a.txt", false, &context)
            .await
            .is_none());
        assert!(appended(&trusted, "read", "web/a.txt", true, &context)
            .await
            .is_none());
        let untrusted = hook(None, config(env, false));
        assert!(appended(&untrusted, "read", "web/a.txt", false, &context)
            .await
            .is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_loop_adds_a_directory_once() {
        let env = project().await;
        let provider = FauxProvider::new(
            FauxScript::new()
                .with_tool_call("read", json!({"path": "web/a.txt"}))
                .with_tool_call("read", json!({"path": "web/b.txt"}))
                .with_text("done"),
        );
        let tool_context = ice_tools::ExecutionToolContext::new(env.clone(), Some(env.clone()));
        let options = AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            tools: vec![HarnessTool::new(ice_tools::create_read_tool(
                &tool_context,
                None,
            ))],
            active_tool_names: vec!["read".into()],
            after_tool_call: Some(hook(None, config(env.clone(), true))),
            ..Default::default()
        };
        let harness = AgentHarness::create(options).await.unwrap();
        harness.prompt_text("look", vec![]).await.unwrap();
        assert_eq!(
            env.read_text_file("/proj/web/a.txt", None).await.unwrap(),
            "a"
        );
        let snapshot = format!("{:?}", harness.context_snapshot().await.unwrap().context);
        assert_eq!(
            snapshot.matches("WEB_RULES").count(),
            1,
            "added once, on the first read: {snapshot}"
        );
    }
}
