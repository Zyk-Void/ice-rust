//! Mirrors `packages/agent/src/harness/system-prompt.ts` — composition of the
//! harness system prompt from a base prompt + the skill listing.
//!
//! The TS `system-prompt.ts` file contains only `formatSkillsForSystemPrompt`;
//! that function is implemented in [`crate::skills`] (co-located with the
//! loader for cohesion, per the module doc). This module owns the *composition*
//! helper that assembles the final system string the harness sends to the
//! provider. The section order mirrors pi `buildSystemPrompt` (both the
//! custom-prompt and default-scaffold branches compose in the same order):
//!
//! **base → append → context → skills**
//!
//! - `base`: the caller's `--system-prompt`, a discovered `SYSTEM.md`, or the
//!   built-in default scaffold.
//! - `append`: text appended after the base (pi's `appendSystemPrompt`, fed by
//!   `--append-system-prompt` flags and a discovered `APPEND_SYSTEM.md`).
//! - `context`: the formatted `<project_context>` block
//!   ([`crate::context_files::format_project_context`], which already starts
//!   with the `\n\n` separator).
//! - `skills`: the `<available_skills>` listing
//!   ([`format_skills_for_system_prompt`]), gated to model-visible skills by
//!   the caller (the `read`-tool + `disable_model_invocation` filters live in
//!   the wiring layer, which passes a pre-filtered slice here).
//!
//! The skill *invocation* block (unescaped, full content) is a separate path
//! (invariant §9) handled by [`crate::skills::format_skill_invocation`] and is
//! injected as a user message at invocation time, NOT into the system prompt.

use crate::skills::format_skills_for_system_prompt;
use crate::types::Skill;

/// Compose the harness system prompt. Mirrors the section assembly in
/// `system-prompt.ts::buildSystemPrompt`: `base` (when present), then `append`
/// text, then the `context` block, then the skill `listing` — each separated by
/// a blank line, with empty sections omitted.
///
/// - `base_prompt` may be `None`/empty — omitted from the prefix.
/// - `append` is raw text wrapped as `\n\n{append}` (matching pi's
///   `appendSection = "\n\n" + appendSystemPrompt`).
/// - `context` is a **pre-formatted** block returned by
///   `format_project_context` (it already carries its own leading `\n\n`), so
///   it is appended verbatim when non-empty.
/// - `skills` is the caller-filtered model-visible slice; the listing is
///   `"\n\n"`-joined only when non-empty.
///
/// When everything is empty the result is the empty string (so a caller can
/// interpolate the return value without leaving stray separators).
pub fn compose_system_prompt(
    base_prompt: Option<&str>,
    skills: &[Skill],
    context: Option<&str>,
    append: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(base) = base_prompt.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(base);
    }
    if let Some(extra) = append.map(str::trim).filter(|s| !s.is_empty()) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(extra);
    }
    if let Some(ctx) = context.filter(|s| !s.is_empty()) {
        // `format_project_context` returns a block already prefixed with
        // `\n\n<project_context>`; append verbatim.
        out.push_str(ctx);
    }
    let listing = format_skills_for_system_prompt(skills);
    if !listing.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&listing);
    }
    out
}

/// Bounded live instructions from connected MCP servers. Limits include headers
/// and truncation notices and are measured in UTF-8 bytes, not characters.
pub fn format_mcp_instructions(
    instructions: &std::collections::BTreeMap<String, String>,
) -> String {
    const PER_SERVER: usize = 2048;
    const TOTAL: usize = 8192;
    const NOTICE: &str = "\n[truncated; use /mcp tools <server> for full instructions]";
    let mut output = String::new();
    for (server, text) in instructions {
        let clean = |value: &str| {
            value
                .chars()
                .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
                .collect::<String>()
        };
        let text = clean(text);
        if text.trim().is_empty() {
            continue;
        }
        let server: String = server
            .chars()
            .filter(|ch| !ch.is_control())
            .take(128)
            .collect();
        let header = format!("\n\nMCP server `{server}` instructions:\n");
        let available = PER_SERVER.min(TOTAL - output.len());
        if available <= header.len() + NOTICE.len() {
            break;
        }
        let budget = available - header.len();
        output.push_str(&header);
        if text.len() <= budget {
            output.push_str(&text);
        } else {
            let mut end = budget - NOTICE.len();
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            output.push_str(&text[..end]);
            output.push_str(NOTICE);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_files::ContextFile;
    use crate::types::Skill;

    fn skill(name: &str, desc: &str, file: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: desc.to_string(),
            content: "c".to_string(),
            file_path: file.to_string(),
            disable_model_invocation: None,
        }
    }

    fn ctx(content: &str, path: &str) -> String {
        crate::context_files::format_project_context(&[ContextFile {
            path: std::path::PathBuf::from(path),
            content: content.to_string(),
        }])
    }

    #[test]
    fn base_only_when_no_extras() {
        assert_eq!(
            compose_system_prompt(Some("You are helpful."), &[], None, None),
            "You are helpful."
        );
    }

    #[test]
    fn listing_only_when_no_base() {
        let out = compose_system_prompt(None, &[skill("x", "d", "/x/SKILL.md")], None, None);
        assert!(out.starts_with("The following skills"));
        assert!(out.contains("<name>x</name>"));
    }

    #[test]
    fn base_append_context_skills_in_order() {
        let context = ctx("project notes", "/p/AGENTS.md");
        let out = compose_system_prompt(
            Some("BASE"),
            &[skill("x", "d", "/x/SKILL.md")],
            Some(&context),
            Some("APPENDED"),
        );
        // Order: BASE \n\n APPENDED \n\n <project_context>... \n\n skills.
        let base_end = out.find("APPENDED").unwrap();
        assert!(base_end < out.find("<project_context>").unwrap());
        assert!(out.find("<project_context>").unwrap() < out.find("The following skills").unwrap());
        assert!(out.starts_with("BASE\n\nAPPENDED"));
    }

    #[test]
    fn append_after_base() {
        let out = compose_system_prompt(Some("BASE"), &[], None, Some("APPENDED"));
        assert_eq!(out, "BASE\n\nAPPENDED");
    }

    #[test]
    fn context_appended_with_its_own_separator() {
        let context = ctx("notes", "/p/AGENTS.md");
        let out = compose_system_prompt(Some("BASE"), &[], Some(&context), None);
        assert!(out.starts_with("BASE\n\n<project_context>"));
        assert!(out.ends_with("</project_context>\n"));
    }

    #[test]
    fn mcp_instructions_are_bounded_and_utf8_safe() {
        let instructions = (0..20)
            .map(|index| (format!("server-{index:02}"), "💧".repeat(3000)))
            .collect();
        let output = format_mcp_instructions(&instructions);
        assert!(output.len() <= 8192);
        assert!(output.contains("MCP server `server-00` instructions:"));
        assert!(output.contains("[truncated; use /mcp tools <server>"));
        for section in output.split("\n\n").filter(|part| !part.is_empty()) {
            assert!(section.len() + 2 <= 2048);
        }
        assert!(!output.contains("server-19"));
    }

    #[test]
    fn mcp_instructions_strip_terminal_controls_and_skip_empty_sections() {
        let instructions = [
            ("a\u{001b}".into(), "Hello\u{001b}[31m\nWorld\t!".into()),
            ("empty".into(), " \n\t".into()),
        ]
        .into();
        let output = format_mcp_instructions(&instructions);
        assert_eq!(
            output,
            "\n\nMCP server `a` instructions:\nHello[31m\nWorld\t!"
        );
        assert!(format_mcp_instructions(&Default::default()).is_empty());
    }

    #[tokio::test]
    async fn mcp_instructions_survive_base_reload_without_accumulating() {
        let harness = crate::agent_harness::AgentHarness::create(Default::default())
            .await
            .unwrap();
        harness
            .set_system_prompt(Some("BASE".into()))
            .await
            .unwrap();
        harness
            .set_mcp_instructions([("demo".into(), "Use echo.".into())].into())
            .await
            .unwrap();
        let snapshot = harness.context_snapshot().await.unwrap();
        assert_eq!(snapshot.base_system_prompt.as_deref(), Some("BASE"));
        assert!(snapshot
            .system_prompt
            .ends_with("MCP server `demo` instructions:\nUse echo."));
        harness
            .set_system_prompt(Some("RELOADED".into()))
            .await
            .unwrap();
        harness
            .set_mcp_instructions([("demo".into(), "Updated.".into())].into())
            .await
            .unwrap();
        let prompt = harness.context_snapshot().await.unwrap().system_prompt;
        assert!(prompt.starts_with("RELOADED"));
        assert!(!prompt.contains("Use echo."));
        assert_eq!(prompt.matches("MCP server `demo`").count(), 1);
        harness
            .set_mcp_instructions(Default::default())
            .await
            .unwrap();
        assert_eq!(
            harness.context_snapshot().await.unwrap().system_prompt,
            "RELOADED"
        );
    }

    #[test]
    fn empty_when_no_base_and_no_visible_skills() {
        let mk = || {
            let mut s = skill("hidden", "h", "/x/SKILL.md");
            s.disable_model_invocation = Some(true);
            s
        };
        assert_eq!(compose_system_prompt(None, &[mk()], None, None), "");
        assert_eq!(compose_system_prompt(Some("   "), &[mk()], None, None), "");
    }
}
