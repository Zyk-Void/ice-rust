//! Consecutive read/search/list calls share one expandable transcript cell.
//! Port of ice `tool-group.ts` and the exploration part of `tool-summary.ts`.
use std::any::Any;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::utils::truncate_to_width;
use crate::{
    bold, theme, BashExecutionComponent, BashStatus, Component, ToolExecutionComponent, ToolStatus,
};

enum Member {
    Tool(Arc<ToolExecutionComponent>),
    Bash(Arc<BashExecutionComponent>),
}

impl Member {
    fn component(&self) -> &dyn Component {
        match self {
            Self::Tool(tool) => tool.as_ref(),
            Self::Bash(bash) => bash.as_ref(),
        }
    }

    fn status(&self) -> ToolStatus {
        match self {
            Self::Tool(tool) => tool.status(),
            Self::Bash(bash) => match bash.status() {
                BashStatus::Running => ToolStatus::Running,
                BashStatus::Complete => ToolStatus::Completed,
                BashStatus::Cancelled | BashStatus::Error => ToolStatus::Failed,
            },
        }
    }

    fn set_expanded(&self, expanded: bool) {
        match self {
            Self::Tool(tool) => tool.set_expanded(expanded),
            Self::Bash(bash) => bash.set_expanded(expanded),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Tool(tool) => describe_exploration(
                &tool.name(),
                &serde_json::from_str(&tool.args()).unwrap_or(Value::Null),
            )
            .unwrap_or_else(|| tool.name()),
            Self::Bash(bash) => {
                describe_exploration("bash", &serde_json::json!({"command": bash.command()}))
                    .unwrap_or_else(|| bash.command())
            }
        }
    }
}

/// Layout only: the original tool components continue to receive all updates.
/// The existing expand-tools action opens every member, including finished ones.
pub struct ToolGroupComponent {
    members: Mutex<Vec<Member>>,
    expanded: Mutex<bool>,
}

impl ToolGroupComponent {
    pub fn new(expanded: bool) -> Self {
        Self {
            members: Mutex::new(Vec::new()),
            expanded: Mutex::new(expanded),
        }
    }

    pub fn add_tool(&self, tool: Arc<ToolExecutionComponent>) {
        self.add(Member::Tool(tool));
    }

    pub fn add_bash(&self, bash: Arc<BashExecutionComponent>) {
        self.add(Member::Bash(bash));
    }

    fn add(&self, member: Member) {
        member.set_expanded(*self.expanded.lock().unwrap());
        self.members.lock().unwrap().push(member);
    }

    /// Failures remain visible and close the group to subsequent calls.
    pub fn is_open(&self) -> bool {
        !self
            .members
            .lock()
            .unwrap()
            .iter()
            .any(|member| member.status() == ToolStatus::Failed)
    }

    pub fn set_expanded(&self, expanded: bool) {
        *self.expanded.lock().unwrap() = expanded;
        for member in self.members.lock().unwrap().iter() {
            member.set_expanded(expanded);
        }
    }
}

impl Component for ToolGroupComponent {
    fn render(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let members = self.members.lock().unwrap();
        if members.is_empty() {
            return Vec::new();
        }
        if *self.expanded.lock().unwrap() {
            return members
                .iter()
                .flat_map(|member| member.component().render(width))
                .collect();
        }
        let width = width.min(100);
        let colors = theme().colors;
        let running = members
            .iter()
            .any(|member| matches!(member.status(), ToolStatus::Pending | ToolStatus::Running));
        let failed = members
            .iter()
            .any(|member| member.status() == ToolStatus::Failed);
        let (marker, color) = if failed {
            ("✗", colors.error)
        } else if running {
            ("›", colors.muted)
        } else {
            ("✓", colors.success)
        };
        let mut lines = vec![
            String::new(),
            format!(
                "{} {}",
                color.fg(marker),
                bold(if running { "Exploring" } else { "Explored" })
            ),
        ];
        let shown = if members.len() > 6 { 5 } else { members.len() };
        for (index, member) in members.iter().take(shown).enumerate() {
            let prefix = if index == 0 { "  ⎿ " } else { "    " };
            lines.push(format!("{prefix}{}", member.description().escape_debug()));
        }
        if shown < members.len() {
            let keys = crate::get_keybindings()
                .get_keys(crate::keybindings::keys::TOOLS_EXPAND)
                .iter()
                .map(|key| key.display())
                .collect::<Vec<_>>()
                .join("/");
            let hint = if keys.is_empty() {
                String::new()
            } else {
                format!(" ({keys} to expand)")
            };
            lines.push(
                colors
                    .dim
                    .fg(&format!("    … +{} more{hint}", members.len() - shown)),
            );
        }
        for member in members
            .iter()
            .filter(|member| member.status() == ToolStatus::Failed)
        {
            lines.extend(
                member
                    .component()
                    .render(width.saturating_sub(2))
                    .into_iter()
                    .map(|line| format!("  {line}")),
            );
        }
        lines
            .into_iter()
            .map(|line| truncate_to_width(&line, width, "…"))
            .collect()
    }

    fn invalidate(&self) {
        for member in self.members.lock().unwrap().iter() {
            member.component().invalidate();
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Display classification only. Unknown or mutating shell commands stay separate.
pub fn describe_exploration(name: &str, args: &Value) -> Option<String> {
    let text = |key| {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
    };
    let path = text("path");
    match name {
        "read" => {
            let offset = args.get("offset").and_then(Value::as_u64);
            let limit = args.get("limit").and_then(Value::as_u64);
            let range = match (offset, limit) {
                (Some(start), Some(count)) => {
                    format!(":{start}-{}", start.saturating_add(count.saturating_sub(1)))
                }
                (Some(start), None) => format!(":{start}"),
                (None, Some(count)) => format!(":1-{count}"),
                _ => String::new(),
            };
            Some(format!("Read {}{range}", path.unwrap_or("file")))
        }
        "grep" | "find" => {
            let pattern = text("pattern").unwrap_or("");
            Some(match path {
                Some(path) => format!("Search {pattern} in {path}"),
                None => format!("Search {pattern}"),
            })
        }
        "ls" => Some(format!("List {}", path.unwrap_or("."))),
        "bash" => {
            let command = text("command")?;
            let kind = classify_shell(command)?;
            Some(format!(
                "{kind} {}",
                command.lines().next().unwrap_or("").trim()
            ))
        }
        _ => None,
    }
}

fn classify_shell(command: &str) -> Option<&'static str> {
    if command.contains(['>', '`']) || command.contains("$(") || command.contains("<(") {
        return None;
    }
    let mut rank = 0;
    for part in command.split(['&', '|', ';', '\n']) {
        let words: Vec<_> = part.split_whitespace().collect();
        let Some(program) = words.first() else {
            continue;
        };
        let kind = match *program {
            "cd" => continue,
            "cat" | "head" | "tail" | "wc" | "nl" | "stat" => 2,
            "sed"
                if words.get(1) == Some(&"-n")
                    && words.get(2).is_some_and(|script| {
                        script
                            .trim_matches(['\'', '"'])
                            .strip_suffix('p')
                            .is_some_and(|range| {
                                !range.is_empty()
                                    && range.bytes().all(|byte| {
                                        byte.is_ascii_digit() || matches!(byte, b',' | b'$')
                                    })
                            })
                    })
                    && words[3..].iter().all(|word| !word.starts_with('-')) =>
            {
                2
            }
            "git"
                if matches!(
                    words.get(1).copied(),
                    Some("status" | "log" | "diff" | "show" | "blame")
                ) && !words.iter().any(|word| word.starts_with("--output")) =>
            {
                2
            }
            "rg" | "grep" | "egrep" | "fgrep" | "ag" | "find" | "fd"
                if !words.iter().any(|word| {
                    matches!(
                        *word,
                        "-exec"
                            | "-execdir"
                            | "-ok"
                            | "-okdir"
                            | "-delete"
                            | "-fprint"
                            | "-fprint0"
                            | "-fprintf"
                            | "-fls"
                    ) || word.starts_with("--pre")
                }) =>
            {
                3
            }
            "ls" | "pwd" => 1,
            _ => return None,
        };
        rank = rank.max(kind);
    }
    match rank {
        1 => Some("List"),
        2 => Some("Read"),
        3 => Some("Search"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{strip_ansi, visible_width, BashTruncation};

    fn tool(name: &str, args: Value) -> Arc<ToolExecutionComponent> {
        Arc::new(ToolExecutionComponent::new(name, &args.to_string()))
    }

    // ice compact-tool-rows.test.ts: lists each exploration, keeps failures
    // visible, closes the run, expands full rows, and caps long runs.
    #[test]
    fn exploration_group_updates_members_and_preserves_failed_output() {
        let group = ToolGroupComponent::new(false);
        let read = tool(
            "read",
            serde_json::json!({"path":"AGENTS.md","offset":2,"limit":3}),
        );
        let grep = tool(
            "grep",
            serde_json::json!({"pattern":"createWindow","path":"src"}),
        );
        let bash = Arc::new(BashExecutionComponent::new("ls main"));
        group.add_tool(read.clone());
        group.add_tool(grep.clone());
        group.add_bash(bash.clone());
        read.set_running();
        grep.set_result("match", false);
        bash.set_complete(Some(0), false, BashTruncation::default());
        let rendered = strip_ansi(&group.render(80).join("\n"));
        assert!(rendered.contains("Exploring"));
        assert!(rendered.contains("Read AGENTS.md:2-4"));
        assert!(rendered.contains("Search createWindow in src"));
        assert!(rendered.contains("List ls main"));
        read.set_result("ENOENT", true);
        assert!(!group.is_open());
        assert!(strip_ansi(&group.render(80).join("\n")).contains("ENOENT"));
        group.set_expanded(true);
        assert!(read.is_expanded() && grep.is_expanded() && bash.is_expanded());
        assert!(strip_ansi(&group.render(80).join("\n")).contains("match"));
    }

    #[test]
    fn exploration_group_caps_rows_and_fits_narrow_and_wide_terminals() {
        let group = ToolGroupComponent::new(false);
        for i in 0..9 {
            let member = tool(
                "read",
                serde_json::json!({"path":format!("{i}{}", "界".repeat(120))}),
            );
            member.set_result("done", false);
            group.add_tool(member);
        }
        let lines = group.render(240);
        assert_eq!(lines.len(), 8);
        assert!(strip_ansi(lines.last().unwrap()).contains("+4 more"));
        for width in [0, 1, 2, 4, 20, 80, 240] {
            assert!(group
                .render(width)
                .iter()
                .all(|line| visible_width(line) <= width.min(100)));
        }
    }

    #[test]
    fn exploration_group_escapes_control_characters_in_labels() {
        let group = ToolGroupComponent::new(false);
        let member = tool("read", serde_json::json!({"path":"line\n\u{1b}[31m\t界"}));
        member.set_result("done", false);
        group.add_tool(member);
        let lines: Vec<_> = group
            .render(80)
            .iter()
            .map(|line| strip_ansi(line))
            .collect();
        assert!(lines.iter().all(|line| !line.chars().any(char::is_control)));
        let label = lines.last().unwrap();
        assert!(label.contains("\\n") && label.contains("\\u{1b}") && label.contains("\\t"));
        assert!(label.contains('界'));
        assert!(lines.iter().all(|line| visible_width(line) <= 80));
    }

    #[test]
    fn shell_exploration_is_display_only_and_conservative() {
        let describe =
            |command| describe_exploration("bash", &serde_json::json!({"command":command}));
        assert_eq!(describe("ls main"), Some("List ls main".into()));
        assert!(describe("cd src && rg foo").unwrap().starts_with("Search "));
        assert!(describe("sed -n '1,20p' file")
            .unwrap()
            .starts_with("Read "));
        for command in [
            "rm x",
            "cat x > y",
            "find . -delete",
            "find . -execdir touch changed ;",
            "find . -fprint listing",
            "git diff --output=changes.patch",
            "rg --pre processor pattern",
            "sed -i x y",
            "sed -n 'w output' file",
            "sed -n 's/x/y/e' file",
            "echo $(cat x)",
            "git push",
            "cd x",
        ] {
            assert_eq!(describe(command), None, "{command}");
        }
        assert_eq!(describe_exploration("edit", &Value::Null), None);
    }
}
