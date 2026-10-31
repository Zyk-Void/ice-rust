//! Slash commands shared by the interactive TUI and headless sessions
//! (`--mode rpc`, and through it `--acp`).
//!
//! The TUI keeps its own registry for UI-bound commands; this module owns
//! what both surfaces need: skill expansion, prompt templates, extension
//! command invocation, and the built-ins that make sense without a terminal.

use ice_extensions::RegistrySnapshot;
use ice_harness::types::AgentHarnessResources;
use serde_json::{json, Value};

/// Built-in commands a headless session runs, with an input hint when the
/// command takes arguments. UI-only TUI commands (`/settings`, `/tree`, …)
/// are deliberately absent.
pub(crate) const HEADLESS_BUILTINS: &[(&str, &str, Option<&str>)] = &[
    (
        "compact",
        "Compact the conversation context",
        Some("optional focus instructions"),
    ),
    (
        "model",
        "Show or switch the model for this session",
        Some("provider/model-id"),
    ),
    (
        "thinking",
        "Show or set the thinking level",
        Some("off|minimal|low|medium|high|xhigh|max"),
    ),
    ("tools", "List the active tools", None),
    ("context", "Show context window usage", None),
    ("usage", "Show token usage and cost for this session", None),
    ("session", "Show this session's id", None),
    (
        "name",
        "Set this session's display name",
        Some("display name"),
    ),
    (
        "cognee",
        "Cognee memory status and settings",
        Some("status|mode|…"),
    ),
    (
        "blackhole",
        "Blackhole compaction status and settings",
        Some("status|on|off|…"),
    ),
    ("version", "Show the icers version", None),
    ("changelog", "Show recent release changes", Some("all")),
    ("help", "List available commands", None),
];

/// A resolved `/command` line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Slash {
    /// A [`HEADLESS_BUILTINS`] entry and its argument text.
    Builtin(&'static str, String),
    /// An extension-registered command and its argument text.
    Extension(String, String),
    /// A prompt template and its parsed arguments.
    Template(String, Vec<String>),
    /// `/skill:name` and its optional additional instructions.
    Skill(String, Option<String>),
}

/// Resolve `text` as a slash command. Precedence matches the TUI: built-ins,
/// then extension commands, then prompt templates; `/skill:name` names a
/// skill. Unknown `/…` input returns `None` and is sent to the model as typed.
pub(crate) fn resolve(
    text: &str,
    resources: &AgentHarnessResources,
    snapshot: Option<&RegistrySnapshot>,
) -> Option<Slash> {
    let text = text.trim();
    let rest = text.strip_prefix('/')?;
    let (token, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let args = args.trim();
    if let Some(name) = token.strip_prefix("skill:") {
        let known = resources
            .skills
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|skill| skill.name == name);
        return known.then(|| {
            Slash::Skill(
                name.to_string(),
                (!args.is_empty()).then(|| args.to_string()),
            )
        });
    }
    if let Some((name, _, _)) = HEADLESS_BUILTINS.iter().find(|(name, _, _)| *name == token) {
        return Some(Slash::Builtin(name, args.to_string()));
    }
    let extension = snapshot.and_then(|snapshot| {
        snapshot
            .commands()
            .iter()
            .find(|command| command.name.trim_start_matches('/') == token)
    });
    if extension.is_some() {
        return Some(Slash::Extension(token.to_string(), args.to_string()));
    }
    let template = resources
        .prompt_templates
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|template| template.name == token);
    template.then(|| {
        Slash::Template(
            token.to_string(),
            ice_harness::prompt_templates::parse_command_args(args),
        )
    })
}

/// Every command a headless session accepts, as
/// `{"name","description","source","hint"?}` (names without the leading `/`).
/// Earlier sources shadow later ones with the same name, as in the TUI.
pub(crate) fn list(
    resources: &AgentHarnessResources,
    snapshot: Option<&RegistrySnapshot>,
) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut push = |name: String, description: String, source: &str, hint: Option<&str>| {
        // Menus show one line: skill descriptions are often multi-line prose.
        let description = one_line(&description, 160);
        if seen.insert(name.clone()) {
            let mut entry = json!({"name": name, "description": description, "source": source});
            if let Some(hint) = hint {
                entry["hint"] = json!(hint);
            }
            out.push(entry);
        }
    };
    for (name, description, hint) in HEADLESS_BUILTINS {
        push(name.to_string(), description.to_string(), "builtin", *hint);
    }
    for command in snapshot.map(RegistrySnapshot::commands).unwrap_or_default() {
        push(
            command.name.trim_start_matches('/').to_string(),
            command.description.clone(),
            "extension",
            Some("arguments"),
        );
    }
    for template in resources.prompt_templates.as_deref().unwrap_or_default() {
        let description = template
            .description
            .clone()
            .unwrap_or_else(|| "Expand prompt template".to_string());
        push(
            template.name.clone(),
            description,
            "template",
            Some("arguments"),
        );
    }
    for skill in resources.skills.as_deref().unwrap_or_default() {
        push(
            format!("skill:{}", skill.name),
            skill.description.clone(),
            "skill",
            Some("additional instructions"),
        );
    }
    out
}

/// `text` on one line (whitespace collapsed), cut to `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Call an extension command's native handler and decode its JSON result.
/// `None` when the command is unknown, fails, or panics.
pub(crate) fn invoke_extension_command(
    snapshot: &RegistrySnapshot,
    name: &str,
    args: &str,
) -> Option<Value> {
    let name = name.trim_start_matches('/');
    let command = snapshot
        .commands()
        .iter()
        .find(|c| c.name.trim_start_matches('/') == name)
        .cloned()?;
    let input = command_envelope(name, args)?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut out = ice_plugin_sdk::StbString::empty();
        let rc = (command.handler)(
            ice_plugin_sdk::StbStringRef::from_str(&input),
            &mut out as *mut ice_plugin_sdk::StbString,
            command.user_data,
        );
        // SAFETY: the callback transfers a live output to its producer.
        let out = unsafe {
            ice_plugin_sdk::callbacks::OwnedString::from_raw(
                out,
                Some(ice_extensions::host_free_string),
            )
        };
        (rc == 0).then(|| out.to_string_lossy())
    }))
    .ok()
    .flatten()?;
    serde_json::from_str(&outcome).ok()
}

/// The handler's input, `{"args":…,"command":"/name"}` as the plugin SDK
/// documents it, whether the caller named the command with or without `/`.
fn command_envelope(name: &str, args: &str) -> Option<String> {
    let command = format!("/{}", name.trim_start_matches('/'));
    serde_json::to_string(&json!({ "args": args, "command": command })).ok()
}

/// Plain-text rendering of an extension command result for a headless client.
/// Interactive results (selector, editor, input) need the TUI.
pub(crate) fn extension_result_text(name: &str, result: Option<Value>) -> String {
    let Some(value) = result else {
        return format!("/{name} failed.");
    };
    match value.get("kind").and_then(Value::as_str) {
        _ if value.is_null() => String::new(),
        Some("message") | None => value
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string()),
        Some(kind) => {
            format!("/{name} opened an interactive {kind} dialog, which needs the icers TUI.")
        }
    }
}

/// Characters a `$name` skill mention may contain.
pub(crate) fn is_skill_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// Expand explicit skill invocations into their full instruction blocks.
///
/// A prompt starting with `/skill:name args` expands like upstream
/// `AgentSession._expandSkillCommand` (block, blank line, args). Otherwise
/// each `$name` mention that starts a word and names a loaded skill is
/// replaced in place; trailing `.`/`:` is read as punctuation. Unknown names
/// (`$PATH`, `$5`, `/skill:missing`) are left as typed.
pub(crate) fn expand_skill_references(
    resources: &ice_harness::types::AgentHarnessResources,
    prompt: &str,
) -> String {
    let skills = resources.skills.as_deref().unwrap_or_default();
    let find = |name: &str| skills.iter().find(|s| s.name == name);
    if let Some(rest) = prompt.strip_prefix("/skill:") {
        let (name, args) = rest.split_once(' ').unwrap_or((rest, ""));
        return match find(name) {
            Some(skill) => ice_harness::skills::format_skill_invocation(skill, Some(args.trim())),
            None => prompt.to_string(),
        };
    }
    let mut expanded = String::new();
    let mut copied = 0;
    for (start, _) in prompt.match_indices('$') {
        if start < copied
            || prompt[..start]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
        {
            continue;
        }
        let rest = &prompt[start + 1..];
        let len = rest.find(|c| !is_skill_name_char(c)).unwrap_or(rest.len());
        let name = rest[..len].trim_end_matches(['.', ':']);
        if let Some(skill) = find(name) {
            expanded.push_str(&prompt[copied..start]);
            expanded.push_str(&ice_harness::skills::format_skill_invocation(skill, None));
            copied = start + 1 + name.len();
        }
    }
    expanded.push_str(&prompt[copied..]);
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_commands_are_named_with_a_slash_as_in_the_tui() {
        let envelope =
            |name| serde_json::from_str::<Value>(&command_envelope(name, "x").unwrap()).unwrap();
        assert_eq!(envelope("pet"), json!({"args": "x", "command": "/pet"}));
        assert_eq!(envelope("/pet"), envelope("pet"));
    }
}
