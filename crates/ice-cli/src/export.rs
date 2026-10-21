//! Session export functionality.
//!
//! Supports Markdown and HTML display exports plus raw-entry JSONL archives.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;

use ice_harness::agent_harness::AgentHarness;
use ice_harness::session::types::{BranchBounds, Entry, EntryOrder, EntryQuery};

/// Export format options.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportFormat {
    Markdown,
    Html,
    /// Compatibility format for a raw-entry JSONL archive.
    ///
    /// Each line is a standalone serialized `Entry`; there is no session header,
    /// mutation envelope, or storage metadata, so this is not reopenable by
    /// `JsonlSessionRepo`. Use native v4 session persistence for resumable files.
    Jsonl,
}

/// Export a native JSONL v4 session file to the specified format.
///
/// The input is parsed with the harness codec. Markdown and HTML render only
/// the active `main` branch; the compatibility JSONL output is a raw-entry
/// archive, not a resumable session file.
pub fn export_file(input: &Path, output: &Path) -> Result<(), String> {
    // Determine format from output extension
    let format = if output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
    {
        ExportFormat::Html
    } else if output
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
    {
        ExportFormat::Jsonl
    } else {
        ExportFormat::Markdown
    };

    let session = read_jsonl_session_entries(input)?;

    match format {
        ExportFormat::Markdown => {
            let entries = active_main_branch(&session.entries, session.main_leaf.as_deref())?;
            export_markdown(&entries, output)
        }
        ExportFormat::Html => {
            let entries = active_main_branch(&session.entries, session.main_leaf.as_deref())?;
            export_html(&entries, output)
        }
        ExportFormat::Jsonl => export_raw_entries_jsonl(&session.entries, output),
    }
}

struct ParsedJsonlSession {
    entries: Vec<Entry>,
    main_leaf: Option<String>,
}

/// Parse a JSONL v4 session file's entries and current `main` leaf.
fn read_jsonl_session_entries(input: &Path) -> Result<ParsedJsonlSession, String> {
    use ice_harness::session::jsonl::parse_mutation;
    use ice_harness::session::types::SessionMutation;

    let content = std::fs::read_to_string(input)
        .map_err(|e| format!("Could not read input file {}: {e}", input.display()))?;

    let mut entries = Vec::new();
    let mut main_leaf = None;
    let mut saw_header = false;
    for (index, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !saw_header {
            // A valid session file starts with the v4 header line. Validate it
            // so a plain JSON/old file fails with a clear message.
            ice_harness::session::jsonl::codec::parse_header(line).map_err(|e| {
                format!(
                    "{} is not a JSONL v4 session file (line {}): {e}",
                    input.display(),
                    index + 1
                )
            })?;
            saw_header = true;
            continue;
        }
        let mutation = parse_mutation(line).map_err(|e| {
            format!(
                "could not parse mutation at {}:{}: {e}",
                input.display(),
                index + 1
            )
        })?;
        match mutation {
            SessionMutation::Entry { entry, lane, .. } => {
                if lane.as_deref() == Some("main") {
                    main_leaf = Some(entry.id().to_owned());
                }
                entries.push(entry);
            }
            SessionMutation::Lane {
                lane: mutation_lane,
                leaf_id,
                ..
            } if mutation_lane == "main" => main_leaf = leaf_id,
            _ => {}
        }
    }
    if !saw_header {
        return Err(format!("{} is empty", input.display()));
    }
    Ok(ParsedJsonlSession { entries, main_leaf })
}

/// Return the active main-lane ancestry in root-to-leaf order.
fn active_main_branch(entries: &[Entry], leaf_id: Option<&str>) -> Result<Vec<Entry>, String> {
    let Some(mut current_id) = leaf_id else {
        return Ok(Vec::new());
    };

    let mut entries_by_id = HashMap::with_capacity(entries.len());
    for entry in entries {
        if entries_by_id.insert(entry.id(), entry).is_some() {
            return Err(format!(
                "Session contains duplicate entry id: {}",
                entry.id()
            ));
        }
    }

    let mut branch = Vec::new();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current_id) {
            return Err(format!(
                "Active main branch contains a parent cycle at {current_id}"
            ));
        }
        let entry = entries_by_id
            .get(current_id)
            .ok_or_else(|| format!("Active main branch references missing entry: {current_id}"))?;
        branch.push((*entry).clone());
        match entry.base().parent_id.as_deref() {
            Some(parent_id) => current_id = parent_id,
            None => break,
        }
    }
    branch.reverse();
    Ok(branch)
}

/// Export the session to the specified format.
///
/// Display formats follow the active `main` branch. `Jsonl` remains a
/// compatibility alias for the raw-entry archive API, which includes every
/// stored entry and is not a reopenable session file.
pub async fn export_session(
    harness: &AgentHarness,
    format: ExportFormat,
    output_path: &Path,
) -> Result<(), String> {
    let query = EntryQuery {
        entry_type: None,
        custom_type: None,
        order: Some(EntryOrder::OldestFirst),
        limit: None,
        cursor: None,
    };
    let tree = harness.session().view("main");

    let entries = match &format {
        ExportFormat::Markdown | ExportFormat::Html => tree
            .find_entries_on_branch(&query, &BranchBounds::default())
            .await
            .map_err(|e| format!("Could not read active main branch: {e}"))?,
        ExportFormat::Jsonl => tree
            .find_entries(&query)
            .await
            .map_err(|e| format!("Could not read session entries: {e}"))?,
    };

    match format {
        ExportFormat::Markdown => export_markdown(&entries, output_path),
        ExportFormat::Html => export_html(&entries, output_path),
        ExportFormat::Jsonl => export_raw_entries_jsonl(&entries, output_path),
    }
}

/// Export the active main branch as a new, resumable native JSONL v4 session.
///
/// The export gets a fresh session identity and creation time, with the source
/// session id recorded as `parentSessionId`. Its entries are the active
/// root-to-leaf ancestry only, resequenced from 1 while retaining their entry
/// ids, timestamps, and parent links. The source name and labels for those
/// entries are copied as final facts; operation, queue, and usage records are
/// intentionally omitted so opening the new branch cannot restore unrelated
/// in-flight work. Publication is exclusive: an existing destination (in
/// particular, a destination that is also the source session file) is never
/// replaced.
pub fn export_native_branch_jsonl(
    harness: &AgentHarness,
    cwd: &Path,
    output: &Path,
) -> Result<(), String> {
    let session_jsonl = run_export_future(async { native_branch_jsonl(harness, cwd).await })?;
    persist_native_jsonl_exclusive(output, &session_jsonl)
}

async fn native_branch_jsonl(harness: &AgentHarness, cwd: &Path) -> Result<String, String> {
    use ice_harness::session::jsonl::codec::{encode_header, encode_mutation};
    use ice_harness::session::jsonl::JsonlV4Header;
    use ice_harness::session::types::SessionMutation;
    use std::time::{SystemTime, UNIX_EPOCH};

    let session = harness.session();
    let source_metadata = session
        .get_metadata()
        .await
        .map_err(|error| format!("Could not read source session metadata: {error}"))?;
    let entries = session
        .find_entries_on_branch(
            &EntryQuery {
                entry_type: None,
                custom_type: None,
                order: Some(EntryOrder::OldestFirst),
                limit: None,
                cursor: None,
            },
            &BranchBounds::default(),
        )
        .await
        .map_err(|error| format!("Could not read active main branch: {error}"))?;
    let name = session
        .get_name()
        .await
        .map_err(|error| format!("Could not read source session name: {error}"))?;
    let mut labels = Vec::new();
    for entry in &entries {
        if let Some(label) = session.get_label(entry.id()).await.map_err(|error| {
            format!(
                "Could not read label for active entry {}: {error}",
                entry.id()
            )
        })? {
            labels.push((entry.id().to_owned(), label));
        }
    }

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is before the Unix epoch: {error}"))?
        .as_millis();
    let created_at = i64::try_from(created_at)
        .map_err(|_| "Current timestamp is outside the supported range".to_string())?;
    let header = JsonlV4Header::new(
        uuid::Uuid::now_v7().to_string(),
        created_at,
        cwd.to_string_lossy().into_owned(),
        Some(source_metadata.id),
        None,
        None,
    );

    let mut jsonl = encode_header(&header);
    let mut sequence = 0_u64;
    for mut entry in entries {
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "Native JSONL export has too many entries".to_string())?;
        entry.base_mut().seq = sequence;
        jsonl.push_str(&encode_mutation(&SessionMutation::Entry {
            seq: sequence,
            timestamp: entry.base().timestamp,
            lane: Some("main".to_string()),
            entry,
        }));
    }

    sequence = sequence
        .checked_add(1)
        .ok_or_else(|| "Native JSONL export has too many mutations".to_string())?;
    jsonl.push_str(&encode_mutation(&SessionMutation::FactName {
        seq: sequence,
        name,
    }));
    for (target_id, label) in labels {
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| "Native JSONL export has too many mutations".to_string())?;
        jsonl.push_str(&encode_mutation(&SessionMutation::FactLabel {
            seq: sequence,
            target_id,
            label: Some(label),
        }));
    }
    Ok(jsonl)
}

/// Run the async session reads on a dedicated runtime thread. This synchronous
/// export API may be called from a thread that already has a Tokio runtime, so
/// it must not try to nest `Runtime::block_on` on that same thread.
fn run_export_future<T, F>(future: F) -> Result<T, String>
where
    T: Send,
    F: Future<Output = Result<T, String>> + Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| format!("Could not start export runtime: {error}"))?
                    .block_on(future)
            })
            .join()
            .map_err(|_| "Native JSONL export task panicked".to_string())?
    })
}

/// Stage beside the destination and atomically publish without replacing it.
fn persist_native_jsonl_exclusive(output: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write;

    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Could not stage native JSONL export beside {}: {error}",
            output.display()
        )
    })?;
    staged.write_all(contents.as_bytes()).map_err(|error| {
        format!(
            "Could not write staged native JSONL export for {}: {error}",
            output.display()
        )
    })?;
    staged.as_file().sync_all().map_err(|error| {
        format!(
            "Could not sync staged native JSONL export for {}: {error}",
            output.display()
        )
    })?;
    staged.persist_noclobber(output).map_err(|error| {
        format!(
            "Could not publish native JSONL export to {} without replacing an existing file: {error}",
            output.display()
        )
    })?;
    Ok(())
}

/// Build the ordinary conversation transcript shared by display export and
/// sharing. This follows only the selected main branch and does not redact it.
pub(crate) async fn active_branch_markdown(harness: &AgentHarness) -> Result<String, String> {
    let entries = harness
        .session()
        .view("main")
        .find_entries_on_branch(
            &EntryQuery {
                order: Some(EntryOrder::OldestFirst),
                ..Default::default()
            },
            &BranchBounds::default(),
        )
        .await
        .map_err(|e| format!("Could not read active main branch: {e}"))?;
    Ok(markdown_transcript(&entries))
}

/// Write `contents` to a new file; an existing target is never overwritten.
fn write_new_file(output_path: &Path, contents: &str, what: &str) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "Refusing to overwrite existing {what} export {}; choose another path",
                    output_path.display()
                )
            } else {
                format!("Could not write {what} export: {e}")
            }
        })?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Could not write {what} export: {e}"))
}

fn export_markdown(entries: &[Entry], output_path: &Path) -> Result<(), String> {
    write_new_file(output_path, &markdown_transcript(entries), "markdown")
}

fn markdown_transcript(entries: &[Entry]) -> String {
    let mut md = String::from("# Session\n\n");
    for e in entries {
        if let Entry::Message(me) = e {
            match &me.message {
                ice_agent::AgentMessage::User(u) => {
                    let text = user_message_text(u);
                    md.push_str(&format!("## User\n\n{}\n\n", text));
                }
                ice_agent::AgentMessage::Assistant(a) => {
                    let text = assistant_text(a);
                    if !text.is_empty() {
                        md.push_str(&format!("## Assistant\n\n{}\n\n", text));
                    }
                }
                _ => {}
            }
        } else if let Entry::Compaction(compaction) = e {
            md.push_str(&format!(
                "## Compaction Summary\n\n{}\n\n",
                compaction.summary
            ));
        } else if let Entry::BranchSummary(summary) = e {
            md.push_str(&format!("## Branch Summary\n\n{}\n\n", summary.summary));
        }
    }
    md
}

fn export_html(entries: &[Entry], output_path: &Path) -> Result<(), String> {
    let mut html = String::from(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Session Export</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 2rem; line-height: 1.6; }
        .message { margin-bottom: 2rem; padding: 1rem; border-radius: 8px; }
        .user { background-color: #f0f9ff; border-left: 4px solid #3b82f6; }
        .assistant { background-color: #f3f4f6; border-left: 4px solid #6b7280; }
        .role { font-weight: bold; margin-bottom: 0.5rem; color: #1f2937; }
        pre { background-color: #1e293b; color: #f8fafc; padding: 1rem; border-radius: 6px; overflow-x: auto; }
        code { font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace; }
    </style>
</head>
<body>
    <h1>Session Export</h1>
"#,
    );

    for e in entries {
        if let Entry::Message(me) = e {
            match &me.message {
                ice_agent::AgentMessage::User(u) => {
                    let text = user_message_text(u);
                    html.push_str(&format!(
                        r#"<div class="message user">
    <div class="role">User</div>
    <div>{}</div>
</div>"#,
                        escape_html(&text)
                    ));
                }
                ice_agent::AgentMessage::Assistant(a) => {
                    let text = assistant_text(a);
                    if !text.is_empty() {
                        html.push_str(&format!(
                            r#"<div class="message assistant">
    <div class="role">Assistant</div>
    <div>{}</div>
</div>"#,
                            escape_html(&text)
                        ));
                    }
                }
                _ => {}
            }
        } else if let Entry::Compaction(compaction) = e {
            html.push_str(&format!(
                r#"<div class="message summary">
    <div class="role">Compaction Summary</div>
    <div>{}</div>
</div>"#,
                escape_html(&compaction.summary)
            ));
        } else if let Entry::BranchSummary(summary) = e {
            html.push_str(&format!(
                r#"<div class="message summary">
    <div class="role">Branch Summary</div>
    <div>{}</div>
</div>"#,
                escape_html(&summary.summary)
            ));
        }
    }

    html.push_str("\n</body>\n</html>");

    write_new_file(output_path, &html, "HTML")
}

/// Write standalone serialized entries as a JSONL archive.
///
/// This intentionally omits a v4 header and mutation wrappers; the resulting
/// archive is not a native session file and cannot be reopened by
/// `JsonlSessionRepo`.
pub fn export_raw_entries_jsonl(entries: &[Entry], output_path: &Path) -> Result<(), String> {
    let mut jsonl = String::new();
    for entry in entries {
        let json = serde_json::to_string(entry)
            .map_err(|e| format!("Could not serialize entry to JSON: {e}"))?;
        jsonl.push_str(&json);
        jsonl.push('\n');
    }
    write_new_file(output_path, &jsonl, "JSONL")
}

fn user_message_text(user: &ice_ai::types::UserMessage) -> String {
    match &user.content {
        ice_ai::types::UserContent::Text(s) => s.clone(),
        ice_ai::types::UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|c| match c {
                ice_ai::types::Content::Text(t) => Some(t.text.clone()),
                ice_ai::types::Content::Image(_) => Some("[Image]".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn assistant_text(assistant: &ice_ai::types::AssistantMessage) -> String {
    assistant
        .content
        .iter()
        .filter_map(|c| match c {
            ice_ai::types::Content::Text(t) => Some(t.text.clone()),
            ice_ai::types::Content::Thinking(t) => {
                Some(format!("<thinking>{}</thinking>", t.thinking))
            }
            ice_ai::types::Content::ToolCall(tc) => Some(format!("[Tool Call: {}]", tc.name)),
            ice_ai::types::Content::Image(_) => Some("[Image]".to_string()),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_harness::session::context::{build_session_context, SessionContextBuildOptions};
    use ice_harness::session::jsonl::codec::{encode_header, encode_mutation};
    use ice_harness::session::jsonl::{HeaderKind, JsonlSessionStorage, JsonlV4Header};
    use ice_harness::session::memory::SystemClock;
    use ice_harness::session::session::Session;
    use ice_harness::session::types::{
        CompactionEntry, Entry, EntryBase, LaneRecord, MessageEntry, OperationIntent,
        OperationStartedRecord, ProvisionedEntry, ProvisionedKind, QueueEnqueuedRecord, QueueKind,
        RecordBase, SessionMutation, UsageCause, UsageRecord,
    };
    use ice_harness::session::DefaultIdGenerator;
    use ice_harness::types::AgentHarnessOptions;
    use ice_tools::{FileSystem, OsExecutionEnv};
    use std::sync::Arc;

    fn user_entry(id: &str, seq: u64, parent_id: Option<&str>, text: &str) -> Entry {
        Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".to_string(),
                id: id.to_string(),
                seq,
                parent_id: parent_id.map(str::to_owned),
                timestamp: seq as i64,
            },
            message: ice_agent::AgentMessage::User(ice_ai::types::UserMessage::new(
                ice_ai::types::UserContent::Text(text.to_string()),
                seq as i64,
            )),
            terminate: None,
        })
    }

    fn assistant_entry(id: &str, seq: u64, parent_id: Option<&str>, text: &str) -> Entry {
        let mut assistant = ice_ai::types::AssistantMessage::empty(
            ice_ai::Api::AnthropicMessages,
            "anthropic",
            "m",
            seq as i64,
        );
        assistant.content = vec![ice_ai::types::Content::text(text)];
        Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".to_string(),
                id: id.to_string(),
                seq,
                parent_id: parent_id.map(str::to_owned),
                timestamp: seq as i64,
            },
            message: ice_agent::AgentMessage::Assistant(Box::new(assistant)),
            terminate: None,
        })
    }

    fn user_message(text: &str, timestamp: i64) -> ice_agent::AgentMessage {
        ice_agent::AgentMessage::User(ice_ai::types::UserMessage::new(
            ice_ai::types::UserContent::Text(text.to_string()),
            timestamp,
        ))
    }

    fn assistant_message(text: &str, timestamp: i64) -> ice_agent::AgentMessage {
        let mut assistant = ice_ai::types::AssistantMessage::empty(
            ice_ai::Api::AnthropicMessages,
            "anthropic",
            "m",
            timestamp,
        );
        assistant.content = vec![ice_ai::types::Content::text(text)];
        ice_agent::AgentMessage::Assistant(Box::new(assistant))
    }

    async fn file_backed_export_harness(source_path: &Path, cwd: &Path) -> (AgentHarness, String) {
        let env = Arc::new(OsExecutionEnv::with_cwd(cwd.to_path_buf()));
        let fs: Arc<dyn FileSystem> = env;
        let source_path_text = source_path.to_string_lossy().into_owned();
        let storage = JsonlSessionStorage::create(
            fs,
            &source_path_text,
            JsonlV4Header::new(
                "source-session".to_string(),
                1,
                cwd.to_string_lossy().into_owned(),
                None,
                None,
                None,
            ),
            Arc::new(SystemClock),
            Arc::new(DefaultIdGenerator::new()),
        )
        .await
        .unwrap();
        let session = Session::new(Arc::new(storage), None);

        let root_id = session
            .append_message(user_message("root history", 10))
            .await
            .unwrap();
        let abandoned_id = session
            .append_message(assistant_message("abandoned branch only", 11))
            .await
            .unwrap();
        session.move_lane("main", Some(&root_id)).await.unwrap();
        let active_id = session
            .append_message(user_message("active branch", 12))
            .await
            .unwrap();
        let compact_id = session
            .append_entry(
                ProvisionedEntry {
                    id: "retained-compaction".to_string(),
                    kind: ProvisionedKind::Compaction {
                        summary: "retained compaction summary".to_string(),
                        retained_tail: vec![user_message("retained tail message", 13)],
                        tokens_before: 100,
                        details: None,
                        usage: None,
                    },
                },
                "main",
            )
            .await
            .unwrap()
            .id()
            .to_string();
        let tail_id = session
            .append_message(assistant_message("active tail", 14))
            .await
            .unwrap();

        session.set_name(Some("native branch name")).await.unwrap();
        session
            .set_label(&root_id, Some("root label"))
            .await
            .unwrap();
        session
            .set_label(&active_id, Some("active label"))
            .await
            .unwrap();
        session
            .set_label(&compact_id, Some("compaction label"))
            .await
            .unwrap();
        session
            .set_label(&tail_id, Some("tail label"))
            .await
            .unwrap();
        session
            .set_label(&abandoned_id, Some("abandoned label"))
            .await
            .unwrap();

        let harness = AgentHarness::create(AgentHarnessOptions {
            session: session.clone(),
            allow_existing_session: true,
            ..AgentHarnessOptions::default()
        })
        .await
        .unwrap();
        for record in [
            LaneRecord::OperationStarted(OperationStartedRecord {
                base: RecordBase {
                    id: "unrelated-open-operation".to_string(),
                    seq: 0,
                    lane: "main".to_string(),
                    timestamp: 0,
                },
                source_leaf_id: Some(tail_id.clone()),
                intent: OperationIntent::Run {
                    original_prompt: Vec::new(),
                    initial_messages: Vec::new(),
                    system_prompt_override: None,
                    resume_data: None,
                },
            }),
            LaneRecord::QueueEnqueued(QueueEnqueuedRecord {
                base: RecordBase {
                    id: "unrelated-queued-work".to_string(),
                    seq: 0,
                    lane: "main".to_string(),
                    timestamp: 0,
                },
                queue: QueueKind::FollowUp,
                run_id: None,
                target: serde_json::json!({"id": "unrelated-queued-entry"}),
            }),
            LaneRecord::Usage(UsageRecord {
                base: RecordBase {
                    id: "unrelated-usage".to_string(),
                    seq: 0,
                    lane: "main".to_string(),
                    timestamp: 0,
                },
                usage: ice_ai::types::Usage::zero(),
                cause: UsageCause::Adjustment,
                run_id: None,
                entry_id: None,
                attempt: None,
                stop_reason: None,
                tool_call_id: None,
                details: None,
            }),
        ] {
            harness.session().append_record(record).await.unwrap();
        }
        (harness, abandoned_id)
    }

    fn append_entry_mutation(lines: &mut Vec<String>, lane: &str, entry: Entry) {
        lines.push(encode_mutation(&SessionMutation::Entry {
            seq: entry.seq(),
            timestamp: entry.base().timestamp,
            lane: Some(lane.to_string()),
            entry,
        }));
    }

    fn v4_header() -> JsonlV4Header {
        JsonlV4Header {
            kind: HeaderKind,
            version: 4,
            id: "s-export".to_string(),
            created_at: 0,
            cwd: "/tmp".to_string(),
            parent_session_id: None,
            legacy_parent_session_path: None,
            metadata: None,
        }
    }

    #[test]
    fn test_escape_html() {
        assert_eq!(
            escape_html("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"
        );
        assert_eq!(escape_html("Hello & World"), "Hello &amp; World");
    }

    /// Build a real JSONL v4 session file containing one user + one assistant
    /// message, then export it to each format via the `--export` code path.
    #[test]
    fn export_file_renders_a_real_jsonl_session() {
        let dir = tempfile::tempdir().unwrap();
        let session_path = dir.path().join("session.jsonl");

        let header = v4_header();

        let mut lines = vec![encode_header(&header)];
        for (seq, message) in [
            (
                1u64,
                ice_agent::message::AgentMessage::User(ice_ai::types::UserMessage::new(
                    ice_ai::types::UserContent::Text("hello <world>".to_string()),
                    0,
                )),
            ),
            (
                2u64,
                ice_agent::message::AgentMessage::Assistant(Box::new({
                    let mut a = ice_ai::types::AssistantMessage::empty(
                        ice_ai::Api::AnthropicMessages,
                        "anthropic",
                        "m",
                        0,
                    );
                    a.content = vec![ice_ai::types::Content::Text(ice_ai::types::TextContent {
                        kind: ice_ai::types::TextContentType,
                        text: "hi there".to_string(),
                        text_signature: None,
                    })];
                    a
                })),
            ),
        ] {
            let entry = Entry::Message(MessageEntry {
                base: EntryBase {
                    entry_type: "message".to_string(),
                    id: format!("e{seq}"),
                    seq,
                    parent_id: (seq > 1).then(|| "e1".to_string()),
                    timestamp: 0,
                },
                message,
                terminate: None,
            });
            lines.push(encode_mutation(&SessionMutation::Entry {
                seq,
                timestamp: 0,
                lane: Some("main".to_string()),
                entry,
            }));
        }
        std::fs::write(&session_path, lines.concat()).unwrap();

        // Markdown
        let md_path = dir.path().join("out.md");
        export_file(&session_path, &md_path).unwrap();
        let md = std::fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("hello <world>"), "{md}");
        assert!(md.contains("hi there"), "{md}");

        // HTML escapes the angle brackets in the user text.
        let html_path = dir.path().join("out.html");
        export_file(&session_path, &html_path).unwrap();
        let html = std::fs::read_to_string(&html_path).unwrap();
        assert!(html.contains("&lt;world&gt;"), "{html}");
        assert!(html.contains("hi there"), "{html}");
        assert!(html.trim_end().ends_with("</html>"), "{html}");

        // JSONL re-emits the parsed entries (not the original wrapper lines).
        let jsonl_path = dir.path().join("out.jsonl");
        export_file(&session_path, &jsonl_path).unwrap();
        let jsonl = std::fs::read_to_string(&jsonl_path).unwrap();
        assert!(!jsonl.contains("\"kind\":\"header\""), "{jsonl}");
        let parsed: Vec<Entry> = jsonl
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert!(read_jsonl_session_entries(&jsonl_path).is_err());
    }

    #[test]
    fn export_file_display_formats_use_active_main_branch_and_keep_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let session_path = dir.path().join("forked-session.jsonl");
        let mut lines = vec![encode_header(&v4_header())];
        append_entry_mutation(
            &mut lines,
            "main",
            user_entry("root", 1, None, "root history"),
        );
        append_entry_mutation(
            &mut lines,
            "main",
            assistant_entry("abandoned", 2, Some("root"), "abandoned branch"),
        );
        lines.push(encode_mutation(&SessionMutation::Lane {
            seq: 3,
            lane: "main".to_string(),
            leaf_id: Some("root".to_string()),
        }));
        append_entry_mutation(
            &mut lines,
            "main",
            user_entry("active", 4, Some("root"), "active branch"),
        );
        append_entry_mutation(
            &mut lines,
            "main",
            Entry::Compaction(CompactionEntry {
                base: EntryBase {
                    entry_type: "compaction".to_string(),
                    id: "compact".to_string(),
                    seq: 5,
                    parent_id: Some("active".to_string()),
                    timestamp: 5,
                },
                summary: "retained context marker".to_string(),
                retained_tail: Vec::new(),
                tokens_before: 100,
                details: None,
                usage: None,
            }),
        );
        append_entry_mutation(
            &mut lines,
            "main",
            assistant_entry("tail", 6, Some("compact"), "after compaction"),
        );
        std::fs::write(&session_path, lines.concat()).unwrap();

        let md_path = dir.path().join("active.md");
        export_file(&session_path, &md_path).unwrap();
        let markdown = std::fs::read_to_string(md_path).unwrap();
        for expected in [
            "root history",
            "active branch",
            "retained context marker",
            "after compaction",
        ] {
            assert!(
                markdown.contains(expected),
                "missing {expected:?}: {markdown}"
            );
        }
        assert!(!markdown.contains("abandoned branch"), "{markdown}");

        let html_path = dir.path().join("active.html");
        export_file(&session_path, &html_path).unwrap();
        let html = std::fs::read_to_string(html_path).unwrap();
        for expected in [
            "root history",
            "active branch",
            "retained context marker",
            "after compaction",
        ] {
            assert!(html.contains(expected), "missing {expected:?}: {html}");
        }
        assert!(!html.contains("abandoned branch"), "{html}");
        assert!(html.contains("Compaction Summary"), "{html}");
    }

    #[tokio::test]
    async fn export_session_display_uses_active_branch_while_jsonl_is_raw_tree_archive() {
        let dir = tempfile::tempdir().unwrap();
        let harness = AgentHarness::create(AgentHarnessOptions {
            allow_existing_session: true,
            ..AgentHarnessOptions::default()
        })
        .await
        .unwrap();
        let session = harness.session();

        let root_id = session
            .append_message(ice_agent::AgentMessage::User(
                ice_ai::types::UserMessage::new(
                    ice_ai::types::UserContent::Text("root branch message".to_string()),
                    1,
                ),
            ))
            .await
            .unwrap();
        session
            .append_message(ice_agent::AgentMessage::Assistant(Box::new({
                let mut assistant = ice_ai::types::AssistantMessage::empty(
                    ice_ai::Api::AnthropicMessages,
                    "anthropic",
                    "m",
                    2,
                );
                assistant.content = vec![ice_ai::types::Content::text("abandoned branch reply")];
                assistant
            })))
            .await
            .unwrap();
        session.move_lane("main", Some(&root_id)).await.unwrap();
        session
            .append_message(ice_agent::AgentMessage::User(
                ice_ai::types::UserMessage::new(
                    ice_ai::types::UserContent::Text("active branch message".to_string()),
                    3,
                ),
            ))
            .await
            .unwrap();
        session
            .append_entry(
                ProvisionedEntry {
                    id: "active-compaction-marker".to_string(),
                    kind: ProvisionedKind::Compaction {
                        summary: "active compaction summary".to_string(),
                        retained_tail: Vec::new(),
                        tokens_before: 100,
                        details: None,
                        usage: None,
                    },
                },
                "main",
            )
            .await
            .unwrap();
        session
            .append_message(ice_agent::AgentMessage::Assistant(Box::new({
                let mut assistant = ice_ai::types::AssistantMessage::empty(
                    ice_ai::Api::AnthropicMessages,
                    "anthropic",
                    "m",
                    4,
                );
                assistant.content = vec![ice_ai::types::Content::text("active tail reply")];
                assistant
            })))
            .await
            .unwrap();

        let md_path = dir.path().join("live.md");
        export_session(&harness, ExportFormat::Markdown, &md_path)
            .await
            .unwrap();
        let markdown = std::fs::read_to_string(md_path).unwrap();
        assert_eq!(active_branch_markdown(&harness).await.unwrap(), markdown);
        for expected in [
            "root branch message",
            "active branch message",
            "active compaction summary",
            "active tail reply",
        ] {
            assert!(
                markdown.contains(expected),
                "missing {expected:?}: {markdown}"
            );
        }
        assert!(!markdown.contains("abandoned branch reply"), "{markdown}");

        let html_path = dir.path().join("live.html");
        export_session(&harness, ExportFormat::Html, &html_path)
            .await
            .unwrap();
        let html = std::fs::read_to_string(html_path).unwrap();
        assert!(html.contains("active compaction summary"), "{html}");
        assert!(!html.contains("abandoned branch reply"), "{html}");

        let archive_path = dir.path().join("raw-entries.jsonl");
        export_session(&harness, ExportFormat::Jsonl, &archive_path)
            .await
            .unwrap();
        let archive = std::fs::read_to_string(archive_path).unwrap();
        assert!(!archive.contains("\"kind\":\"header\""), "{archive}");
        assert!(archive.contains("abandoned branch reply"), "{archive}");
        assert_eq!(archive.lines().count(), 5);
    }

    #[tokio::test]
    async fn native_export_round_trips_only_the_active_branch_as_a_fresh_v4_session() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source-session.jsonl");
        let output_path = dir.path().join("native-branch.jsonl");
        let (harness, abandoned_id) = file_backed_export_harness(&source_path, dir.path()).await;
        let source_session = harness.session();
        let original_branch = source_session
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..EntryQuery::default()
                },
                &BranchBounds::default(),
            )
            .await
            .unwrap();
        let source_bytes = std::fs::read(&source_path).unwrap();

        export_native_branch_jsonl(&harness, dir.path(), &output_path).unwrap();

        let exported = std::fs::read_to_string(&output_path).unwrap();
        let mut lines = exported.lines();
        let header = ice_harness::session::jsonl::parse_header(lines.next().unwrap()).unwrap();
        assert_eq!(header.version, 4);
        assert_ne!(header.id, "source-session");
        assert!(uuid::Uuid::parse_str(&header.id).is_ok(), "{}", header.id);
        assert!(header.created_at > 1);
        assert_eq!(header.parent_session_id.as_deref(), Some("source-session"));
        assert!(exported.contains("native branch name"));
        assert!(exported.contains("tail label"));
        assert!(!exported.contains("abandoned label"));
        assert!(!exported.contains("unrelated-open-operation"));
        assert!(!exported.contains("unrelated-queued-work"));
        assert!(!exported.contains("unrelated-usage"));

        let env = Arc::new(OsExecutionEnv::with_cwd(dir.path().to_path_buf()));
        let fs: Arc<dyn FileSystem> = env;
        let output_path_text = output_path.to_string_lossy().into_owned();
        let loaded = JsonlSessionStorage::load(
            fs,
            &output_path_text,
            Arc::new(SystemClock),
            Arc::new(DefaultIdGenerator::new()),
        )
        .await
        .unwrap();
        let loaded_metadata = loaded.jsonl_metadata();
        assert_eq!(
            loaded_metadata.parent_session_id.as_deref(),
            Some("source-session")
        );
        let loaded_session = Session::new(Arc::new(loaded), None);
        let entries = loaded_session
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..EntryQuery::default()
                },
                &BranchBounds::default(),
            )
            .await
            .unwrap();

        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries.iter().map(Entry::id).collect::<Vec<_>>(),
            original_branch.iter().map(Entry::id).collect::<Vec<_>>()
        );
        assert_eq!(
            entries.iter().map(Entry::seq).collect::<Vec<_>>(),
            (1..=entries.len() as u64).collect::<Vec<_>>()
        );
        for (exported_entry, source_entry) in entries.iter().zip(&original_branch) {
            assert_eq!(
                exported_entry.base().timestamp,
                source_entry.base().timestamp
            );
            assert_eq!(
                exported_entry.base().parent_id,
                source_entry.base().parent_id
            );
        }
        assert_eq!(
            loaded_session.get_name().await.unwrap().as_deref(),
            Some("native branch name")
        );
        for (entry, label) in entries.iter().zip([
            "root label",
            "active label",
            "compaction label",
            "tail label",
        ]) {
            assert_eq!(
                loaded_session
                    .get_label(entry.id())
                    .await
                    .unwrap()
                    .as_deref(),
                Some(label)
            );
        }
        assert_eq!(loaded_session.get_label(&abandoned_id).await.unwrap(), None);
        assert!(loaded_session
            .find_records(&ice_harness::session::types::RecordQuery::default())
            .await
            .unwrap()
            .is_empty());

        let context = build_session_context(&entries, &SessionContextBuildOptions::default());
        let context_json = serde_json::to_string(&context.messages).unwrap();
        for expected in [
            "retained compaction summary",
            "retained tail message",
            "active tail",
        ] {
            assert!(
                context_json.contains(expected),
                "missing {expected}: {context_json}"
            );
        }
        assert!(
            !context_json.contains("abandoned branch only"),
            "{context_json}"
        );
        assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    }

    #[tokio::test]
    async fn native_export_refuses_source_path_without_modifying_it() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source-session.jsonl");
        let (harness, _) = file_backed_export_harness(&source_path, dir.path()).await;
        let source_bytes = std::fs::read(&source_path).unwrap();

        let error = export_native_branch_jsonl(&harness, dir.path(), &source_path).unwrap_err();

        assert!(
            error.contains("without replacing an existing file"),
            "{error}"
        );
        assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    }

    #[test]
    fn export_file_rejects_a_non_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("not-a-session.jsonl");
        std::fs::write(&bogus, "{\"type\":\"message\"}\n").unwrap();
        let out = dir.path().join("out.md");
        let err = export_file(&bogus, &out).unwrap_err();
        assert!(err.contains("not a JSONL v4 session file"), "{err}");
    }

    #[test]
    fn display_and_archive_exports_refuse_to_overwrite_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.md");
        std::fs::write(&target, "precious").unwrap();
        for result in [
            export_markdown(&[], &target),
            export_html(&[], &target),
            export_raw_entries_jsonl(&[], &target),
        ] {
            assert!(result.unwrap_err().contains("Refusing to overwrite"));
        }
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "precious");
        let fresh = dir.path().join("fresh.md");
        export_markdown(&[], &fresh).unwrap();
        assert!(fresh.exists());
    }
}
