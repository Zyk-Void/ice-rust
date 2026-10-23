//! Settings-driven command hooks. Commands are trusted local code, not a sandbox.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ice_agent::hooks::{AfterToolCall, BeforeToolCall};
use ice_agent::types::{AfterToolCallResult, BeforeToolCallResult};
use ice_agent::{AgentMessage, TextContentOrImage};
use ice_ai::types::{Content, TextContent, UserContent};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub type HookSettings = BTreeMap<String, Vec<MatcherGroup>>;

#[derive(Clone, Debug, Deserialize)]
pub struct MatcherGroup {
    #[serde(default)]
    pub matcher: Option<String>,
    #[serde(default)]
    pub hooks: Vec<CommandHook>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CommandHook {
    #[serde(rename = "type")]
    pub kind: String,
    pub command: String,
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Default, Debug)]
struct Decision {
    deny: Option<String>,
    updated_input: Option<Value>,
    context: Vec<String>,
}

/// A frozen, trust-filtered settings snapshot for one session.
pub struct Hooks {
    settings: HookSettings,
    cwd: PathBuf,
    session_id: String,
    pre_context: Mutex<BTreeMap<String, Vec<String>>>,
    pending_context: Mutex<Vec<String>>,
}

impl Hooks {
    pub fn is_empty(&self) -> bool {
        self.settings
            .values()
            .all(|groups| groups.iter().all(|group| group.hooks.is_empty()))
    }

    pub fn load(cwd: &Path, session_id: String, global: &HookSettings, trusted: bool) -> Arc<Self> {
        let mut settings = global.clone();
        if trusted {
            for project in crate::settings::load_project_settings(cwd) {
                for (event, groups) in project.hooks {
                    settings.entry(event).or_default().extend(groups);
                }
            }
        }
        Arc::new(Self {
            settings,
            cwd: cwd.into(),
            session_id,
            pre_context: Mutex::default(),
            pending_context: Mutex::default(),
        })
    }

    fn payload(&self, event: &str) -> Value {
        json!({"hook_event_name":event,"session_id":self.session_id,"cwd":self.cwd})
    }

    async fn run(&self, event: &str, mut payload: Value, signal: CancellationToken) -> Decision {
        let mut result = Decision::default();
        for group in self.settings.get(event).into_iter().flatten() {
            let matcher = group.matcher.as_deref().unwrap_or("");
            if !matcher.is_empty() && matcher != "*" {
                match regex::Regex::new(matcher) {
                    Ok(regex) if regex.is_match(payload["tool_name"].as_str().unwrap_or("")) => {}
                    Ok(_) => continue,
                    Err(error) => {
                        tracing::warn!(%error, "invalid hook matcher");
                        continue;
                    }
                }
            }
            for hook in &group.hooks {
                if hook.kind != "command" || signal.is_cancelled() {
                    continue;
                }
                let hook = hook.clone();
                let cwd = self.cwd.clone();
                let input = payload.to_string();
                let id = self.session_id.clone();
                let cancel = signal.clone();
                let output =
                    tokio::task::spawn_blocking(move || execute(&hook, &cwd, &id, input, cancel))
                        .await;
                let output = match output {
                    Ok(Ok(output)) => output,
                    other => {
                        tracing::warn!(?other, "command hook failed (ignored)");
                        continue;
                    }
                };
                if output.0 == Some(2) {
                    result.deny = Some(String::from_utf8_lossy(&output.2).trim().to_owned());
                } else if output.0 == Some(0) {
                    if let Ok(value) = serde_json::from_slice::<Value>(&output.1) {
                        let specific = &value["hookSpecificOutput"];
                        if specific["permissionDecision"] == "deny" || value["decision"] == "block"
                        {
                            result.deny = Some(
                                specific["permissionDecisionReason"]
                                    .as_str()
                                    .or_else(|| value["reason"].as_str())
                                    .unwrap_or("Blocked by command hook")
                                    .into(),
                            );
                        }
                        if let Some(updated) = specific.get("updatedInput") {
                            result.updated_input = Some(updated.clone());
                            payload["tool_input"] = updated.clone();
                        }
                        if let Some(context) = specific["additionalContext"]
                            .as_str()
                            .or_else(|| value["additionalContext"].as_str())
                        {
                            result.context.push(context.into());
                        }
                    }
                } else {
                    tracing::warn!(status = ?output.0, stderr = %String::from_utf8_lossy(&output.2), "command hook exited unsuccessfully (ignored)");
                }
                if result.deny.is_some() {
                    break;
                }
            }
            if result.deny.is_some() {
                break;
            }
        }
        if event != "PreToolUse" && event != "UserPromptSubmit" {
            if let Some(reason) = &result.deny {
                tracing::warn!(%event, %reason, "advisory hook block ignored");
            }
        }
        result
    }

    /// Compose after plugin approval so command hooks cannot undo a plugin veto.
    pub fn before(self: &Arc<Self>, plugin: Option<BeforeToolCall>) -> BeforeToolCall {
        let hooks = self.clone();
        Arc::new(move |ctx, signal| {
            let hooks = hooks.clone();
            let mut payload = hooks.payload("PreToolUse");
            payload["tool_name"] = json!(ctx.tool_call.name);
            payload["tool_input"] = ctx.args.clone();
            let id = ctx.tool_call.id.clone();
            let plugin_future = plugin.as_ref().map(|plugin| plugin(ctx, signal.clone()));
            Box::pin(async move {
                let mut base = match plugin_future {
                    Some(future) => future.await.unwrap_or_default(),
                    None => BeforeToolCallResult::default(),
                };
                if base.block {
                    return Some(base);
                }
                if let Some(args) = &base.args {
                    payload["tool_input"] = args.clone();
                }
                let decision = hooks.run("PreToolUse", payload, signal).await;
                if !decision.context.is_empty() {
                    hooks
                        .pre_context
                        .lock()
                        .unwrap()
                        .insert(id, decision.context.clone());
                }
                if decision.updated_input.is_some() {
                    base.args = decision.updated_input;
                }
                if let Some(reason) = decision.deny {
                    base.block = true;
                    base.reason = Some(format!("{reason}\n{}", decision.context.join("\n")));
                }
                Some(base)
            })
        })
    }

    pub fn after(self: &Arc<Self>, previous: Option<AfterToolCall>) -> AfterToolCall {
        let hooks = self.clone();
        Arc::new(move |ctx, signal| {
            let hooks = hooks.clone();
            let mut payload = hooks.payload("PostToolUse");
            payload["tool_name"] = json!(ctx.tool_call.name);
            payload["tool_input"] = ctx.args.clone();
            payload["tool_response"] = json!({"content":ctx.result.clone().into_content(),"details":ctx.result.details,"is_error":ctx.is_error});
            let id = ctx.tool_call.id.clone();
            let original = ctx.result.content.clone();
            let previous_future = previous
                .as_ref()
                .map(|previous| previous(ctx, signal.clone()));
            Box::pin(async move {
                let mut base = match previous_future {
                    Some(future) => future.await.unwrap_or_default(),
                    None => AfterToolCallResult::default(),
                };
                let decision = hooks.run("PostToolUse", payload, signal).await;
                let mut context = hooks
                    .pre_context
                    .lock()
                    .unwrap()
                    .remove(&id)
                    .unwrap_or_default();
                context.extend(decision.context);
                if !context.is_empty() {
                    let content = base.content.get_or_insert(original);
                    content.push(TextContentOrImage::text(context.join("\n")));
                }
                Some(base)
            })
        })
    }

    /// Drain advisory lifecycle context at the next model boundary.
    pub fn transform(self: &Arc<Self>) -> ice_agent::hooks::TransformContext {
        let hooks = self.clone();
        Arc::new(move |mut messages, _| {
            let context = std::mem::take(&mut *hooks.pending_context.lock().unwrap());
            Box::pin(async move {
                if !context.is_empty() {
                    messages.push(AgentMessage::User(ice_ai::types::UserMessage::new(
                        context.join("\n"),
                        0,
                    )));
                }
                messages
            })
        })
    }

    pub async fn start(&self) -> Vec<String> {
        let mut payload = self.payload("SessionStart");
        payload["source"] = json!("startup");
        self.run("SessionStart", payload, CancellationToken::new())
            .await
            .context
    }
}

#[async_trait::async_trait]
impl ice_harness::types::HarnessLifecycleHooks for Hooks {
    async fn input(
        &self,
        mut message: AgentMessage,
    ) -> ice_harness::result::HarnessResult<AgentMessage> {
        if let AgentMessage::User(user) = &mut message {
            let prompt = match &user.content {
                UserContent::Text(text) => text.clone(),
                UserContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        Content::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            let mut payload = self.payload("UserPromptSubmit");
            payload["prompt"] = json!(prompt);
            let decision = self
                .run("UserPromptSubmit", payload, CancellationToken::new())
                .await;
            if let Some(reason) = decision.deny {
                return Err(ice_harness::result::HarnessError::io(reason));
            }
            if !decision.context.is_empty() {
                let context = decision.context.join("\n");
                match &mut user.content {
                    UserContent::Text(text) => {
                        text.push('\n');
                        text.push_str(&context);
                    }
                    UserContent::Blocks(blocks) => blocks.push(Content::Text(TextContent {
                        kind: ice_ai::types::TextContentType,
                        text: context,
                        text_signature: None,
                    })),
                }
            }
        }
        Ok(message)
    }
    async fn pre_compact(&self, trigger: &str) {
        let mut payload = self.payload("PreCompact");
        payload["trigger"] = json!(trigger);
        let decision = self
            .run("PreCompact", payload, CancellationToken::new())
            .await;
        self.pending_context
            .lock()
            .unwrap()
            .extend(decision.context);
    }
    async fn stop(&self) {
        let decision = self
            .run("Stop", self.payload("Stop"), CancellationToken::new())
            .await;
        self.pending_context
            .lock()
            .unwrap()
            .extend(decision.context);
        self.pre_context.lock().unwrap().clear();
    }
}

// Inherit only basic shell/OS environment. In particular no provider keys,
// auth tokens, proxy credentials, or arbitrary ICERS_* variables are inherited.
fn execute(
    hook: &CommandHook,
    cwd: &Path,
    id: &str,
    input: String,
    signal: CancellationToken,
) -> Result<(Option<i32>, Vec<u8>, Vec<u8>), String> {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", &hook.command]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", &hook.command]);
        command
    };
    command.env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "TERM",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("ICERS_PROJECT_DIR", cwd)
        .env("ICERS_SESSION_ID", id)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ice_plugin_sdk::process::ProcessTree::prepare(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let tree = match ice_plugin_sdk::process::ProcessTree::attach(&child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    fn drain(mut pipe: impl Read) -> Vec<u8> {
        let mut result = Vec::new();
        let mut buffer = [0; 8192];
        while let Ok(count) = pipe.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let keep = count.min((1024 * 1024usize).saturating_sub(result.len()));
            result.extend_from_slice(&buffer[..keep]);
        }
        result
    }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || drain(stdout));
    let err = std::thread::spawn(move || drain(stderr));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.code()),
            Ok(None) => {}
            Err(error) => break Err(error.to_string()),
        }
        if signal.is_cancelled()
            || start.elapsed() >= Duration::from_secs(hook.timeout.unwrap_or(60))
        {
            break Err("command hook timed out or was cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    tree.kill();
    let _ = child.wait();
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    Ok((status?, stdout, stderr))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::agent_harness::{AgentHarness, AgentLane};
    use ice_harness::types::{AgentHarnessOptions, HarnessLifecycleHooks, HarnessTool};

    fn configured(dir: &Path, event: &str, command: &str) -> Arc<Hooks> {
        let script = dir.join("hook.sh");
        std::fs::write(&script, command).unwrap();
        let settings = serde_json::from_value(json!({event:[{"hooks":[{"type":"command","command":format!("sh '{}'", script.display())}]}]})).unwrap();
        Hooks::load(dir, "test-session".into(), &settings, false)
    }

    #[tokio::test]
    async fn decisions_matchers_and_exit_codes() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = configured(dir.path(), "PreToolUse", "cat >/dev/null; printf '%s' '{\"hookSpecificOutput\":{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"no\"}}'");
        let mut payload = hooks.payload("PreToolUse");
        payload["tool_name"] = json!("bash");
        assert_eq!(
            hooks
                .run("PreToolUse", payload.clone(), CancellationToken::new())
                .await
                .deny
                .as_deref(),
            Some("no")
        );
        for (exit, blocked) in [(0, false), (1, false), (2, true), (127, false)] {
            let hooks = configured(
                dir.path(),
                "PreToolUse",
                &format!("cat >/dev/null; printf reason >&2; exit {exit}"),
            );
            assert_eq!(
                hooks
                    .run("PreToolUse", payload.clone(), CancellationToken::new())
                    .await
                    .deny
                    .is_some(),
                blocked
            );
        }
        let mut hooks = configured(
            dir.path(),
            "PreToolUse",
            "echo '{\"decision\":\"block\",\"reason\":\"blocked\"}'",
        );
        Arc::get_mut(&mut hooks)
            .unwrap()
            .settings
            .get_mut("PreToolUse")
            .unwrap()[0]
            .matcher = Some("^read$".into());
        assert!(hooks
            .run("PreToolUse", payload.clone(), CancellationToken::new())
            .await
            .deny
            .is_none());
        payload["tool_name"] = json!("read");
        assert!(hooks
            .run("PreToolUse", payload, CancellationToken::new())
            .await
            .deny
            .is_some());
    }

    #[tokio::test]
    async fn timeout_kills_descendants_and_does_not_block_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut hooks = configured(dir.path(), "Stop", "(sleep 2; touch survived) & wait");
        Arc::get_mut(&mut hooks)
            .unwrap()
            .settings
            .get_mut("Stop")
            .unwrap()[0]
            .hooks[0]
            .timeout = Some(1);
        let start = Instant::now();
        let runner = hooks.run("Stop", hooks.payload("Stop"), CancellationToken::new());
        let ticker = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(start.elapsed() < Duration::from_millis(500));
        };
        tokio::join!(runner, ticker);
        assert!(start.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!dir.path().join("survived").exists());
    }

    #[tokio::test]
    async fn project_hooks_require_trust_and_concatenate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".icers")).unwrap();
        std::fs::write(dir.path().join(".icers/settings.json"), r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo '{\"additionalContext\":\"project\"}'"}]}]}}"#).unwrap();
        let global = configured(
            dir.path(),
            "SessionStart",
            "echo '{\"additionalContext\":\"global\"}'",
        );
        assert_eq!(global.start().await, vec!["global"]);
        let trusted = Hooks::load(dir.path(), "test".into(), &global.settings, true);
        assert_eq!(trusted.start().await, vec!["global", "project"]);
    }

    #[tokio::test]
    async fn environment_and_payload_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = configured(
            dir.path(),
            "SessionStart",
            "cat >payload.json; env >environment.txt",
        );
        hooks.start().await;
        let payload: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("payload.json")).unwrap())
                .unwrap();
        assert_eq!(payload["hook_event_name"], "SessionStart");
        assert_eq!(payload["session_id"], "test-session");
        let environment = std::fs::read_to_string(dir.path().join("environment.txt")).unwrap();
        for line in environment.lines() {
            let key = line.split('=').next().unwrap();
            assert!(
                !key.contains("TOKEN") && !key.contains("API_KEY") && !key.contains("AUTH"),
                "{key}"
            );
        }
        assert!(environment.contains("ICERS_SESSION_ID=test-session"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_loop_rewrites_injects_context_and_preserves_plugin_veto() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = configured(dir.path(), "PreToolUse", "cat >pre.json; echo '{\"hookSpecificOutput\":{\"updatedInput\":{\"path\":\"rewritten.txt\",\"content\":\"rewritten\"},\"additionalContext\":\"pre context\"}}'");
        let mut hooks = hooks;
        Arc::get_mut(&mut hooks).unwrap().settings.insert(
            "PostToolUse".into(),
            vec![MatcherGroup {
                matcher: None,
                hooks: vec![CommandHook {
                    kind: "command".into(),
                    command: "cat >post.json; echo '{\"additionalContext\":\"post context\"}'"
                        .into(),
                    timeout: None,
                }],
            }],
        );
        let provider = FauxProvider::new(
            FauxScript::new()
                .with_tool_call("write", json!({"path":"original.txt","content":"original"}))
                .with_text("done"),
        );
        let env = Arc::new(ice_tools::InMemoryExecutionEnv::new());
        let context = ice_tools::ExecutionToolContext::new(env.clone(), Some(env.clone()));
        let options = AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            tools: vec![HarnessTool::new(ice_tools::create_write_tool(&context))],
            active_tool_names: vec!["write".into()],
            before_tool_call: Some(hooks.before(None)),
            after_tool_call: Some(hooks.after(None)),
            ..Default::default()
        };
        let harness = AgentHarness::create(options).await.unwrap();
        harness.prompt_text("write", vec![]).await.unwrap();
        use ice_tools::FileSystem;
        assert_eq!(
            env.read_text_file("rewritten.txt", None).await.unwrap(),
            "rewritten"
        );
        assert!(env.read_text_file("original.txt", None).await.is_err());
        let snapshot = harness.context_snapshot().await.unwrap();
        assert!(format!("{snapshot:?}").contains("pre context"));
        assert!(format!("{snapshot:?}").contains("post context"));
        let post: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("post.json")).unwrap()).unwrap();
        assert_eq!(post["tool_input"]["path"], "rewritten.txt");
        assert!(post["tool_response"]["content"].is_array());
        let veto: BeforeToolCall = Arc::new(|_, _| {
            Box::pin(async {
                Some(BeforeToolCallResult {
                    block: true,
                    reason: Some("plugin veto".into()),
                    ..Default::default()
                })
            })
        });
        // Plugin veto is tested through the adapter, with a complete call context.
        let assistant =
            ice_ai::providers::faux::faux_assistant_message("", ice_ai::types::StopReason::ToolUse);
        let call = ice_ai::types::ToolCall {
            kind: ice_ai::types::ToolCallType,
            id: "veto".into(),
            name: "write".into(),
            arguments: json!({}),
            thought_signature: None,
            namespace: None,
        };
        let context = ice_agent::types::AgentContext::default();
        let result = hooks.before(Some(veto))(
            ice_agent::types::BeforeToolCallContext {
                assistant_message: &assistant,
                tool_call: &call,
                args: &call.arguments,
                context: &context,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(result.block);
        assert_eq!(result.reason.as_deref(), Some("plugin veto"));
    }

    #[tokio::test]
    async fn command_hook_denial_precedes_mcp_approval() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = configured(dir.path(), "PreToolUse", "echo '{\"hookSpecificOutput\":{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"blocked by hook\"}}'");
        let ui = ice_extensions::UiDialogMailbox::new();
        let before = crate::mcp::approval_hook(
            Some(hooks.before(None)),
            Arc::new(Mutex::new(std::collections::HashSet::from([
                "fake_echo".into()
            ]))),
            ui.clone(),
        );
        let assistant =
            ice_ai::providers::faux::faux_assistant_message("", ice_ai::types::StopReason::ToolUse);
        let call = ice_ai::types::ToolCall {
            kind: ice_ai::types::ToolCallType,
            id: "denied".into(),
            name: "fake_echo".into(),
            arguments: json!({}),
            thought_signature: None,
            namespace: None,
        };
        let context = ice_agent::types::AgentContext::default();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            before(
                ice_agent::types::BeforeToolCallContext {
                    assistant_message: &assistant,
                    tool_call: &call,
                    args: &call.arguments,
                    context: &context,
                },
                CancellationToken::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.block);
        assert!(result.reason.unwrap().contains("blocked by hook"));
        assert_eq!(ui.pending_len(), 0);
        assert!(ui.take_pending().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prompt_admission_context_and_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = configured(
            dir.path(),
            "UserPromptSubmit",
            "echo '{\"additionalContext\":\"prompt context\"}'",
        );
        let provider = FauxProvider::new(FauxScript::new().with_text("done"));
        let harness = AgentHarness::create(AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            ..Default::default()
        })
        .await
        .unwrap();
        harness.set_lifecycle_hooks(hooks);
        harness.prompt_text("hello", vec![]).await.unwrap();
        assert!(
            format!("{:?}", harness.context_snapshot().await.unwrap()).contains("prompt context")
        );
        let hooks = configured(dir.path(), "UserPromptSubmit", "echo rejected >&2; exit 2");
        harness.set_lifecycle_hooks(hooks.clone());
        let error = harness
            .prompt_text("must not persist", vec![])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("rejected"));
        assert!(!format!("{:?}", harness.context_snapshot().await.unwrap())
            .contains("must not persist"));
        hooks.pre_compact("manual").await;
    }
}
