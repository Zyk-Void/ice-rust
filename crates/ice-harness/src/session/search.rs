//! Session search — mirrors `packages/agent/src/harness/session/search-backend.ts`.
//!
//! Provides full-text search across session entries. Searches user and assistant
//! message content, tool calls, and custom entries. Returns ranked results with
//! context snippets.
//!
//! # Use Cases
//!
//! - **Session picker**: Search across all sessions to find a specific conversation.
//! - **Content discovery**: Find where a topic was discussed across sessions.
//! - **Audit**: Locate specific tool calls or decisions in session history.
//!
//! # Example
//!
//! ```rust,no_run
//! use ice_harness::session::search::{SessionSearch, SessionSearchOptions};
//! use ice_harness::session::types::{SessionMetadata, Entry};
//! use std::collections::HashMap;
//!
//! # async fn example() {
//! let search = SessionSearch::new();
//! let sessions: HashMap<String, (SessionMetadata, Vec<Entry>)> = HashMap::new();
//!
//! let results = search.search(
//!     &sessions,
//!     "database migration",
//!     SessionSearchOptions {
//!         limit: Some(10),
//!         include_tool_calls: true,
//!         include_custom_entries: false,
//!     }
//! ).await;
//!
//! for hit in results {
//!     println!("Session {}: {}", hit.session_id, hit.snippet);
//! }
//! # }
//! ```
//! ```

use std::collections::HashMap;
use std::fmt::Display;

use async_trait::async_trait;
use ice_agent::message::AgentMessage;
use ice_ai::types::Content;

use super::types::{Entry, EntryOrder, EntryQuery, SessionMetadata, SessionRepo, SessionStorage};
use crate::error::{SessionError, SessionResult};

/// A storage view used by [`ScanningSessionSearch`]. Implementations must return
/// entries according to the supplied query, including the requested order.
#[async_trait]
pub trait SessionSearchStorage: Send + Sync {
    async fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>>;
}

#[async_trait]
impl<T: SessionStorage> SessionSearchStorage for T {
    async fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>> {
        SessionStorage::find_entries(self, query).await
    }
}

/// Source abstraction for complete serialized-entry scans. `Metadata` is kept
/// generic so backend-specific fields (such as JSONL `cwd`, `path`, and opaque
/// metadata) are preserved in hits. `cwd` returns the backend's exact cwd value;
/// `None` means the metadata type does not carry one.
#[async_trait]
pub trait SessionSearchSource: Send + Sync {
    type Metadata: Clone + Send + Sync;
    type Storage: SessionSearchStorage;

    async fn list(&self) -> SessionResult<Vec<Self::Metadata>>;
    async fn open(&self, metadata: &Self::Metadata) -> SessionResult<Self::Storage>;
    fn cwd<'a>(metadata: &'a Self::Metadata) -> Option<&'a str>;
}

/// Search adapter for the shared [`SessionRepo`] API. Base Rust metadata does
/// not include a cwd, so a requested cwd never matches through this adapter.
pub struct SessionRepoSearchSource<'a, R> {
    repo: &'a R,
}

impl<'a, R> SessionRepoSearchSource<'a, R> {
    pub fn new(repo: &'a R) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl<R> SessionSearchSource for SessionRepoSearchSource<'_, R>
where
    R: SessionRepo,
    R::Storage: SessionSearchStorage,
{
    type Metadata = SessionMetadata;
    type Storage = R::Storage;

    async fn list(&self) -> SessionResult<Vec<Self::Metadata>> {
        self.repo.list().await
    }

    async fn open(&self, metadata: &Self::Metadata) -> SessionResult<Self::Storage> {
        self.repo.open(metadata).await
    }

    fn cwd(_: &Self::Metadata) -> Option<&str> {
        None
    }
}

/// Search adapter retaining JSONL's richer metadata and exact cwd filtering.
pub struct JsonlSessionRepoSearchSource<'a> {
    repo: &'a super::jsonl::JsonlSessionRepo,
}

impl<'a> JsonlSessionRepoSearchSource<'a> {
    pub fn new(repo: &'a super::jsonl::JsonlSessionRepo) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl SessionSearchSource for JsonlSessionRepoSearchSource<'_> {
    type Metadata = super::jsonl::JsonlSessionMetadata;
    type Storage = super::jsonl::JsonlSessionStorage;

    async fn list(&self) -> SessionResult<Vec<Self::Metadata>> {
        self.repo
            .list_typed(&super::jsonl::JsonlSessionListOptions::default())
            .await
    }

    async fn open(&self, metadata: &Self::Metadata) -> SessionResult<Self::Storage> {
        self.repo.open_by_jsonl_metadata(metadata).await
    }

    fn cwd(metadata: &Self::Metadata) -> Option<&str> {
        Some(metadata.cwd.as_str())
    }
}

/// Options for a repository scan over all serialized entry fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySessionSearchOptions {
    /// Query text; leading/trailing whitespace is ignored and matching is
    /// case-insensitive. An empty normalized query performs no repository I/O.
    pub text: String,
    /// Optional exact working-directory filter. Metadata without a cwd cannot
    /// match when this is supplied.
    pub cwd: Option<String>,
}

/// A hit from [`ScanningSessionSearch`]. Unlike ranked [`SessionSearchHit`],
/// repository-scan hits are returned in source/entry order and have no score.
#[derive(Debug, Clone, PartialEq)]
pub struct RepositorySessionSearchHit<TMetadata> {
    pub metadata: TMetadata,
    pub entry_id: String,
    /// ISO-8601 UTC timestamp using JavaScript `Date.toISOString()` format.
    pub timestamp: String,
    /// The entire serialized entry, not a context-truncated excerpt.
    pub snippet: String,
}

/// Unranked scanner matching the TypeScript `createScanningSessionSearch`
/// contract: source-list order, oldest-first entries, and the whole serialized
/// entry as both searchable payload and result snippet.
pub struct ScanningSessionSearch<S> {
    source: S,
}

impl<S> ScanningSessionSearch<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

/// Construct the TypeScript-compatible all-entry scanner from a search source.
pub fn create_scanning_session_search<S: SessionSearchSource>(
    source: S,
) -> ScanningSessionSearch<S> {
    ScanningSessionSearch::new(source)
}

impl<S: SessionSearchSource> ScanningSessionSearch<S> {
    /// Search complete serialized entries without ranking, limiting, or
    /// truncating results. Source and storage errors pass through unchanged;
    /// serialization failures become `storage` errors and invalid JavaScript
    /// timestamps become `invalid_entry` errors.
    pub async fn search(
        &self,
        options: RepositorySessionSearchOptions,
    ) -> SessionResult<Vec<RepositorySessionSearchHit<S::Metadata>>> {
        let normalized_query = options.text.trim().to_lowercase();
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits = Vec::new();
        for metadata in self.source.list().await? {
            if let Some(cwd) = options.cwd.as_deref() {
                if S::cwd(&metadata) != Some(cwd) {
                    continue;
                }
            }

            let storage = self.source.open(&metadata).await?;
            let entries = storage
                .find_entries(&EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..EntryQuery::default()
                })
                .await?;
            for entry in entries {
                let snippet = serialize_entry(&entry)?;
                if !snippet.to_lowercase().contains(&normalized_query) {
                    continue;
                }
                let timestamp =
                    super::jsonl::repo::js_iso_timestamp_from_ms(entry.base().timestamp)?;
                hits.push(RepositorySessionSearchHit {
                    metadata: metadata.clone(),
                    entry_id: entry.id().to_string(),
                    timestamp,
                    snippet,
                });
            }
        }
        Ok(hits)
    }
}

/// Options for a session search.
#[derive(Debug, Clone, Default)]
pub struct SessionSearchOptions {
    /// Maximum number of results to return. `None` means unlimited.
    pub limit: Option<usize>,
    /// Whether to include tool call names and arguments in the search.
    pub include_tool_calls: bool,
    /// Whether to include custom entry data in the search.
    pub include_custom_entries: bool,
}

/// A search result hit.
#[derive(Debug, Clone)]
pub struct SessionSearchHit {
    /// The session id where the hit was found.
    pub session_id: String,
    /// The entry id where the hit was found.
    pub entry_id: String,
    /// The role of the message (user, assistant, or system).
    pub role: String,
    /// A snippet of text around the match.
    pub snippet: String,
    /// The position of the match in the full text (character offset).
    pub match_position: usize,
    /// Relevance score (higher is better).
    pub score: f64,
}

/// Session search engine.
pub struct SessionSearch;

impl SessionSearch {
    /// Create a new session search engine.
    pub fn new() -> Self {
        Self
    }

    /// Search across multiple sessions for a query string.
    ///
    /// # Arguments
    ///
    /// * `sessions` - Map of session metadata to session entries.
    /// * `query` - The search query (case-insensitive).
    /// * `options` - Search options (limit, what to include, etc.).
    ///
    /// # Returns
    ///
    /// A list of search hits, sorted by relevance score (highest first).
    pub async fn search(
        &self,
        sessions: &HashMap<String, (SessionMetadata, Vec<Entry>)>,
        query: &str,
        options: SessionSearchOptions,
    ) -> Vec<SessionSearchHit> {
        let query_lower = query.trim().to_lowercase();
        if query_lower.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();

        for (session_id, (_metadata, entries)) in sessions {
            for entry in entries {
                if let Some(hit) = self.search_entry(
                    session_id,
                    entry,
                    &query_lower,
                    options.include_tool_calls,
                    options.include_custom_entries,
                ) {
                    hits.push(hit);
                }
            }
        }

        // Sort by score (highest first)
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply limit
        if let Some(limit) = options.limit {
            hits.truncate(limit);
        }

        hits
    }

    /// Search a single entry for the query.
    fn search_entry(
        &self,
        session_id: &str,
        entry: &Entry,
        query_lower: &str,
        include_tool_calls: bool,
        include_custom_entries: bool,
    ) -> Option<SessionSearchHit> {
        match entry {
            Entry::Message(msg_entry) => {
                let role = match &msg_entry.message {
                    AgentMessage::User(_) => "user",
                    AgentMessage::Assistant(_) => "assistant",
                    AgentMessage::ToolResult(_) => "tool_result",
                    AgentMessage::Custom(_) => return None,
                };

                let text = self.extract_message_text(&msg_entry.message, include_tool_calls);
                self.find_match(session_id, &msg_entry.base.id, role, &text, query_lower)
            }
            Entry::Custom(custom_entry) if include_custom_entries => {
                let text = serde_json::to_string(&custom_entry.data).unwrap_or_default();
                self.find_match(
                    session_id,
                    &custom_entry.base.id,
                    "custom",
                    &text,
                    query_lower,
                )
            }
            _ => None,
        }
    }

    /// Extract searchable text from a message.
    fn extract_message_text(&self, message: &AgentMessage, include_tool_calls: bool) -> String {
        match message {
            AgentMessage::User(user_msg) => {
                // User messages can be text or blocks
                match &user_msg.content {
                    ice_ai::types::UserContent::Text(text) => text.clone(),
                    ice_ai::types::UserContent::Blocks(blocks) => blocks
                        .iter()
                        .filter_map(|block| match block {
                            Content::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                }
            }
            AgentMessage::Assistant(assistant_msg) => {
                let mut parts: Vec<String> = Vec::new();
                for content in &assistant_msg.content {
                    match content {
                        Content::Text(t) => parts.push(t.text.clone()),
                        Content::ToolCall(tc) if include_tool_calls => {
                            parts.push(tc.name.clone());
                            // Tool-call arguments are JSON; include their string form so
                            // searches can match on argument values too.
                            if let Ok(args_str) = serde_json::to_string(&tc.arguments) {
                                parts.push(args_str);
                            }
                        }
                        _ => {}
                    }
                }
                parts.join("\n")
            }
            AgentMessage::ToolResult(tr) => {
                let mut parts = Vec::new();
                for content in &tr.content {
                    match content {
                        Content::Text(t) => parts.push(t.text.as_str()),
                        _ => {}
                    }
                }
                parts.join("\n")
            }
            AgentMessage::Custom(c) => serde_json::to_string(&c.data).unwrap_or_default(),
        }
    }

    /// Find a match in the text and create a hit.
    fn find_match(
        &self,
        session_id: &str,
        entry_id: &str,
        role: &str,
        text: &str,
        query_lower: &str,
    ) -> Option<SessionSearchHit> {
        let (text_lower, original_char_offsets) = lowercase_with_char_offsets(text);
        let position = text_lower.find(query_lower)?;
        let match_end = position + query_lower.len();
        let match_char_position = original_char_offsets[position];
        // A query may match only part of a lowercase expansion (for example,
        // `i` in the lowercase form of `İ`). Include its original character.
        let match_char_end = original_char_offsets[match_end - 1] + 1;
        let snippet = self.extract_snippet(text, match_char_position, match_char_end);

        // Keep the original ranking model, but express position in characters
        // rather than UTF-8 bytes so Unicode does not skew its position factor.
        let match_count = text_lower.matches(query_lower).count();
        let character_count = text.chars().count().max(1);
        let position_factor = 1.0 - (match_char_position as f64 / character_count as f64).min(1.0);
        let score = (match_count as f64) * (0.5 + 0.5 * position_factor);

        Some(SessionSearchHit {
            session_id: session_id.to_string(),
            entry_id: entry_id.to_string(),
            role: role.to_string(),
            snippet,
            match_position: match_char_position,
            score,
        })
    }

    /// Extract a snippet with up to 100 Unicode characters on either side of
    /// the matched source characters.
    fn extract_snippet(&self, text: &str, match_start: usize, match_end: usize) -> String {
        let char_boundaries: Vec<usize> = text
            .char_indices()
            .map(|(byte_offset, _)| byte_offset)
            .chain(std::iter::once(text.len()))
            .collect();
        let character_count = char_boundaries.len() - 1;
        let start = match_start.saturating_sub(100);
        let end = match_end.saturating_add(100).min(character_count);
        let byte_start = char_boundaries[start];
        let byte_end = char_boundaries[end];

        let mut snippet = String::new();
        if start > 0 {
            snippet.push_str("...");
        }
        snippet.push_str(&text[byte_start..byte_end]);
        if end < character_count {
            snippet.push_str("...");
        }
        snippet
    }
}

fn serialize_entry(entry: &Entry) -> SessionResult<String> {
    serialize_entry_with(entry, serde_json::to_string)
}

fn serialize_entry_with<E: Display>(
    entry: &Entry,
    serialize: impl FnOnce(&Entry) -> Result<String, E>,
) -> SessionResult<String> {
    serialize(entry).map_err(|error| {
        SessionError::storage(format!(
            "Failed to serialize session entry {}: {error}",
            entry.id()
        ))
    })
}

/// Lowercase text while recording each output byte's originating source
/// character. Lowercase mappings can expand one Unicode character into several
/// (e.g. `İ`), so byte offsets in the normalized text are not source offsets.
fn lowercase_with_char_offsets(text: &str) -> (String, Vec<usize>) {
    // Lowercase the whole string so context-sensitive mappings (notably the
    // Greek final sigma) agree with the query and raw-entry scanner. Unicode's
    // language-neutral contextual mapping changes the character, not its UTF-8
    // width; per-character expansion widths still map bytes back to the source.
    let lowered = text.to_lowercase();
    let mut original_char_offsets = Vec::with_capacity(lowered.len() + 1);
    for (char_offset, original_char) in text.chars().enumerate() {
        for lower_char in original_char.to_lowercase() {
            let mut encoded = [0; 4];
            let lower_char = lower_char.encode_utf8(&mut encoded);
            original_char_offsets.extend(std::iter::repeat(char_offset).take(lower_char.len()));
        }
    }
    original_char_offsets.push(text.chars().count());
    (lowered, original_char_offsets)
}

impl Default for SessionSearch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::TextContentType;
    use ice_ai::types::{AssistantMessage, TextContent, UserContent, UserMessage};

    use crate::session::jsonl::{
        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    use crate::session::memory::{CounterIdGenerator, FakeClock};
    use crate::session::types::{
        CustomEntry, EntryBase, MessageEntry, ModelChangeEntry, ProvisionedEntry, ProvisionedKind,
        SessionStorage,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn create_user_entry(id: &str, text: &str) -> Entry {
        Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".to_string(),
                id: id.to_string(),
                seq: 0,
                timestamp: 0,
                parent_id: None,
            },
            message: AgentMessage::User(UserMessage {
                role: Default::default(),
                content: UserContent::Text(text.to_string()),
                timestamp: 0,
                origin: None,
            }),
            terminate: None,
        })
    }

    fn create_assistant_entry(id: &str, text: &str) -> Entry {
        Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".to_string(),
                id: id.to_string(),
                seq: 0,
                timestamp: 0,
                parent_id: None,
            },
            message: AgentMessage::Assistant(Box::new(AssistantMessage {
                role: Default::default(),
                content: vec![Content::Text(TextContent {
                    kind: TextContentType,
                    text: text.to_string(),
                    text_signature: None,
                })],
                api: ice_ai::types::Api::AnthropicMessages,
                provider: "test".to_string(),
                model: "test-model".to_string(),
                response_model: None,
                response_id: None,
                usage: ice_ai::types::Usage::zero(),
                stop_reason: ice_ai::types::StopReason::Stop,
                deferred: None,
                error_message: None,
                raw_stop_reason: None,
                end_turn: None,
                timestamp: 0,
            })),
            terminate: None,
        })
    }

    fn make_metadata(id: &str) -> SessionMetadata {
        SessionMetadata {
            id: id.to_string(),
            created_at: 0,
            parent_session_id: None,
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct RichMetadata {
        id: String,
        cwd: Option<String>,
        application_field: String,
    }

    #[derive(Clone)]
    struct FakeSearchStorage {
        entries: Vec<Entry>,
        error: Option<SessionError>,
    }

    #[async_trait]
    impl SessionSearchStorage for FakeSearchStorage {
        async fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            let mut entries = self.entries.clone();
            if query.order == Some(EntryOrder::OldestFirst) {
                entries.sort_by_key(Entry::seq);
            }
            Ok(entries)
        }
    }

    #[derive(Clone)]
    struct FakeSearchSource {
        metadata: Vec<RichMetadata>,
        entries: HashMap<String, Vec<Entry>>,
        list_error: Option<SessionError>,
        open_error: Option<SessionError>,
        storage_error: Option<SessionError>,
        list_calls: Arc<AtomicUsize>,
        open_calls: Arc<AtomicUsize>,
    }

    impl FakeSearchSource {
        fn new(metadata: Vec<RichMetadata>, entries: HashMap<String, Vec<Entry>>) -> Self {
            Self {
                metadata,
                entries,
                list_error: None,
                open_error: None,
                storage_error: None,
                list_calls: Arc::new(AtomicUsize::new(0)),
                open_calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl SessionSearchSource for FakeSearchSource {
        type Metadata = RichMetadata;
        type Storage = FakeSearchStorage;

        async fn list(&self) -> SessionResult<Vec<Self::Metadata>> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = &self.list_error {
                return Err(error.clone());
            }
            Ok(self.metadata.clone())
        }

        async fn open(&self, metadata: &Self::Metadata) -> SessionResult<Self::Storage> {
            self.open_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = &self.open_error {
                return Err(error.clone());
            }
            Ok(FakeSearchStorage {
                entries: self.entries.get(&metadata.id).cloned().unwrap_or_default(),
                error: self.storage_error.clone(),
            })
        }

        fn cwd(metadata: &Self::Metadata) -> Option<&str> {
            metadata.cwd.as_deref()
        }
    }

    fn rich_metadata(id: &str, cwd: Option<&str>) -> RichMetadata {
        RichMetadata {
            id: id.to_string(),
            cwd: cwd.map(str::to_string),
            application_field: format!("opaque-{id}"),
        }
    }

    fn create_custom_entry(id: &str, seq: u64, timestamp: i64, data: serde_json::Value) -> Entry {
        Entry::Custom(CustomEntry {
            base: EntryBase {
                entry_type: "custom".to_string(),
                id: id.to_string(),
                seq,
                timestamp,
                parent_id: None,
            },
            custom_type: "test_payload".to_string(),
            data: Some(data),
        })
    }

    fn create_model_change_entry(id: &str, seq: u64, timestamp: i64) -> Entry {
        Entry::ModelChange(ModelChangeEntry {
            base: EntryBase {
                entry_type: "model_change".to_string(),
                id: id.to_string(),
                seq,
                timestamp,
                parent_id: None,
            },
            provider: "provider-needle".to_string(),
            model_id: "model-a".to_string(),
        })
    }

    fn repository_search(source: FakeSearchSource) -> ScanningSessionSearch<FakeSearchSource> {
        create_scanning_session_search(source)
    }

    #[tokio::test]
    async fn repository_scanner_searches_raw_types_and_custom_payload_in_source_order() {
        let second_metadata = rich_metadata("session-z", Some("/repo"));
        let first_metadata = rich_metadata("session-a", Some("/repo"));
        let mut entries = HashMap::new();
        // Deliberately reverse the storage order: each session must be scanned
        // chronologically, while the repository's listed session order wins.
        let newest_entry = create_custom_entry(
            "z-new",
            2,
            1_700_000_000_123,
            serde_json::json!({"nested": {"payload": "needle custom value"}}),
        );
        entries.insert(
            second_metadata.id.clone(),
            vec![
                newest_entry.clone(),
                create_model_change_entry("z-old", 1, 0),
            ],
        );
        entries.insert(
            first_metadata.id.clone(),
            vec![create_custom_entry(
                "a-old",
                1,
                0,
                serde_json::json!({"secret": "needle"}),
            )],
        );
        let scanner = repository_search(FakeSearchSource::new(
            vec![second_metadata.clone(), first_metadata.clone()],
            entries,
        ));

        let hits = scanner
            .search(RepositorySessionSearchOptions {
                text: "  NEEDLE  ".to_string(),
                cwd: None,
            })
            .await
            .unwrap();

        assert_eq!(
            hits.iter()
                .map(|hit| hit.entry_id.as_str())
                .collect::<Vec<_>>(),
            ["z-old", "z-new", "a-old"]
        );
        assert_eq!(hits[0].metadata, second_metadata);
        assert_eq!(hits[0].metadata.application_field, "opaque-session-z");
        assert!(hits[0].snippet.contains("model_change"));
        assert!(hits[1].snippet.contains("needle custom value"));
        assert_eq!(
            hits[1].snippet,
            serde_json::to_string(&newest_entry).unwrap()
        );
        assert_eq!(hits[0].timestamp, "1970-01-01T00:00:00.000Z");
        assert_eq!(hits[1].timestamp, "2023-11-14T22:13:20.123Z");

        let type_hits = scanner
            .search(RepositorySessionSearchOptions {
                text: "MODEL_CHANGE".to_string(),
                cwd: None,
            })
            .await
            .unwrap();
        assert_eq!(type_hits.len(), 1);
        assert_eq!(type_hits[0].entry_id, "z-old");
    }

    #[tokio::test]
    async fn jsonl_repository_adapter_preserves_rich_metadata_and_filters_cwd() {
        let fs: Arc<dyn ice_tools::env::FileSystem> =
            Arc::new(ice_tools::InMemoryExecutionEnv::with_cwd("/work".into()));
        let repo = JsonlSessionRepo::new(
            JsonlSessionRepoOptions {
                fs,
                sessions_root: "/sessions".into(),
                clock: Arc::new(FakeClock::new()),
                ids: Arc::new(CounterIdGenerator::new()),
            },
            "/work".into(),
        );
        let mut application_metadata = serde_json::Map::new();
        application_metadata.insert("applicationField".into(), serde_json::json!("kept"));

        for (id, cwd, text) in [
            ("in-work", "/work", "find-me in working directory"),
            ("other-work", "/other", "find-me elsewhere"),
        ] {
            let storage = repo
                .create_typed(&JsonlSessionCreateOptions {
                    id: Some(id.into()),
                    parent_session_id: None,
                    cwd: cwd.into(),
                    metadata: Some(application_metadata.clone()),
                })
                .await
                .unwrap();
            storage
                .append_entry(
                    ProvisionedEntry {
                        id: format!("{id}-entry"),
                        kind: ProvisionedKind::Message {
                            message: AgentMessage::User(UserMessage::new(text, 0)),
                            terminate: None,
                        },
                    },
                    "main",
                )
                .await
                .unwrap();
        }

        let scanner = create_scanning_session_search(JsonlSessionRepoSearchSource::new(&repo));
        let hits = scanner
            .search(RepositorySessionSearchOptions {
                text: " FIND-ME ".to_string(),
                cwd: Some("/work".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].metadata.id, "in-work");
        assert_eq!(hits[0].metadata.cwd, "/work");
        assert!(hits[0].metadata.path.contains("in-work"));
        assert_eq!(
            hits[0].metadata.metadata.as_ref().unwrap()["applicationField"],
            "kept"
        );
        assert_eq!(hits[0].entry_id, "in-work-entry");
        assert!(hits[0].snippet.contains("find-me in working directory"));

        let base_metadata_scanner =
            create_scanning_session_search(SessionRepoSearchSource::new(&repo));
        let base_hits = base_metadata_scanner
            .search(RepositorySessionSearchOptions {
                text: "find-me".to_string(),
                cwd: None,
            })
            .await
            .unwrap();
        assert_eq!(base_hits.len(), 1);
        assert_eq!(base_hits[0].metadata.id, "in-work");
    }

    #[tokio::test]
    async fn repository_scanner_filters_cwd_by_exact_match() {
        let exact = rich_metadata("exact", Some("/work"));
        let missing = rich_metadata("missing", None);
        let child = rich_metadata("child", Some("/work/subdir"));
        let mut entries = HashMap::new();
        for metadata in [&exact, &missing, &child] {
            entries.insert(
                metadata.id.clone(),
                vec![create_custom_entry(
                    &format!("{}-entry", metadata.id),
                    1,
                    0,
                    serde_json::json!({"payload": "match"}),
                )],
            );
        }
        let source = FakeSearchSource::new(vec![exact.clone(), missing, child], entries);
        let open_calls = source.open_calls.clone();
        let scanner = repository_search(source);

        let hits = scanner
            .search(RepositorySessionSearchOptions {
                text: "match".to_string(),
                cwd: Some("/work".to_string()),
            })
            .await
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].metadata, exact);
        assert_eq!(open_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repository_scanner_blank_query_does_not_read_source() {
        let source =
            FakeSearchSource::new(vec![rich_metadata("s1", Some("/work"))], HashMap::new());
        let list_calls = source.list_calls.clone();
        let open_calls = source.open_calls.clone();
        let scanner = repository_search(source);

        let hits = scanner
            .search(RepositorySessionSearchOptions {
                text: " \t\n ".to_string(),
                cwd: None,
            })
            .await
            .unwrap();

        assert!(hits.is_empty());
        assert_eq!(list_calls.load(Ordering::SeqCst), 0);
        assert_eq!(open_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn repository_scanner_propagates_listing_open_and_storage_errors() {
        let metadata = rich_metadata("s1", Some("/work"));
        let expected = SessionError::storage("search source failed");
        let mut source = FakeSearchSource::new(vec![metadata.clone()], HashMap::new());
        source.list_error = Some(expected.clone());
        let scanner = repository_search(source);
        let error = scanner
            .search(RepositorySessionSearchOptions {
                text: "needle".to_string(),
                cwd: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, expected.code);
        assert_eq!(error.message, expected.message);

        let expected = SessionError::not_found("session disappeared");
        let mut source = FakeSearchSource::new(vec![metadata.clone()], HashMap::new());
        source.open_error = Some(expected.clone());
        let scanner = repository_search(source);
        let error = scanner
            .search(RepositorySessionSearchOptions {
                text: "needle".to_string(),
                cwd: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, expected.code);
        assert_eq!(error.message, expected.message);

        let expected = SessionError::storage("entry read failed");
        let mut source = FakeSearchSource::new(vec![metadata], HashMap::new());
        source.storage_error = Some(expected.clone());
        let scanner = repository_search(source);
        let error = scanner
            .search(RepositorySessionSearchOptions {
                text: "needle".to_string(),
                cwd: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, expected.code);
        assert_eq!(error.message, expected.message);
    }

    #[test]
    fn repository_scanner_converts_serialization_errors_to_storage_errors() {
        let entry = create_user_entry("bad-json", "needle");
        let error = serialize_entry_with(&entry, |_| Err("serializer failed")).unwrap_err();
        assert_eq!(error.code, crate::error::SessionErrorCode::Storage);
        assert!(error.message.contains("serializer failed"));
    }

    #[tokio::test]
    async fn repository_scanner_propagates_invalid_timestamp_errors() {
        let metadata = rich_metadata("s1", Some("/work"));
        let mut invalid_timestamp = create_user_entry("bad-timestamp", "needle");
        invalid_timestamp.base_mut().timestamp = 8_640_000_000_000_001;
        let mut entries = HashMap::new();
        entries.insert(metadata.id.clone(), vec![invalid_timestamp]);
        let scanner = repository_search(FakeSearchSource::new(vec![metadata], entries));
        let error = scanner
            .search(RepositorySessionSearchOptions {
                text: "needle".to_string(),
                cwd: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, crate::error::SessionErrorCode::InvalidEntry);
        assert!(error.message.contains("JavaScript Date range"));
    }

    #[tokio::test]
    async fn search_finds_user_messages() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();

        let metadata = make_metadata("session-1");
        let entries = vec![
            create_user_entry("e1", "Hello world"),
            create_user_entry("e2", "Database migration is important"),
        ];

        sessions.insert("session-1".to_string(), (metadata, entries));

        let results = search
            .search(
                &sessions,
                "database",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "session-1");
        assert_eq!(results[0].entry_id, "e2");
        assert_eq!(results[0].role, "user");
        assert!(results[0].snippet.contains("Database"));
    }

    #[tokio::test]
    async fn search_finds_assistant_messages() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();

        let metadata = make_metadata("session-1");
        let entries = vec![create_assistant_entry(
            "e1",
            "I can help with database migration",
        )];

        sessions.insert("session-1".to_string(), (metadata, entries));

        let results = search
            .search(
                &sessions,
                "migration",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role, "assistant");
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();

        let metadata = make_metadata("session-1");
        let entries = vec![
            create_user_entry("e1", "test match 1"),
            create_user_entry("e2", "test match 2"),
            create_user_entry("e3", "test match 3"),
        ];

        sessions.insert("session-1".to_string(), (metadata, entries));

        let results = search
            .search(
                &sessions,
                "test",
                SessionSearchOptions {
                    limit: Some(2),
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn ranked_search_uses_character_offsets_and_safe_snippets_for_multibyte_text() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();
        sessions.insert(
            "session-1".to_string(),
            (
                make_metadata("session-1"),
                vec![create_user_entry("e1", &format!("{}x", "中".repeat(40)))],
            ),
        );

        let results = search
            .search(
                &sessions,
                "x",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_position, 40);
        assert!(results[0].snippet.contains('中'));
    }

    #[tokio::test]
    async fn ranked_search_maps_lowercase_expansion_offsets_to_original_characters() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();
        sessions.insert(
            "session-1".to_string(),
            (
                make_metadata("session-1"),
                vec![create_user_entry("e1", "İx")],
            ),
        );

        let results = search
            .search(
                &sessions,
                "x",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_position, 1);

        let expansion_match = search
            .search(
                &sessions,
                "i",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;
        assert_eq!(expansion_match.len(), 1);
        assert_eq!(expansion_match[0].match_position, 0);
        assert!(expansion_match[0].snippet.contains('İ'));
    }

    #[tokio::test]
    async fn ranked_search_snippet_context_counts_unicode_characters() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();
        sessions.insert(
            "session-1".to_string(),
            (
                make_metadata("session-1"),
                vec![create_user_entry(
                    "e1",
                    &format!("{}x{}", "前".repeat(120), "後".repeat(120)),
                )],
            ),
        );

        let results = search
            .search(
                &sessions,
                "x",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        let snippet = &results[0].snippet;
        let context = snippet
            .strip_prefix("...")
            .unwrap()
            .strip_suffix("...")
            .unwrap();
        assert_eq!(context.chars().count(), 201);
    }

    #[tokio::test]
    async fn ranked_search_returns_no_hits_for_blank_query() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();
        sessions.insert(
            "session-1".to_string(),
            (
                make_metadata("session-1"),
                vec![create_user_entry("e1", "ordinary content")],
            ),
        );

        let results = search
            .search(
                &sessions,
                " ",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn ranked_search_preserves_context_sensitive_lowercase() {
        let sessions = HashMap::from([(
            "greek".to_string(),
            (
                make_metadata("greek"),
                vec![create_user_entry("sigma", "ΟΣ")],
            ),
        )]);
        let hits = SessionSearch::new()
            .search(&sessions, "ος", SessionSearchOptions::default())
            .await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].match_position, 0);
        assert_eq!(hits[0].snippet, "ΟΣ");
    }

    #[tokio::test]
    async fn search_is_case_insensitive() {
        let search = SessionSearch::new();
        let mut sessions = HashMap::new();

        let metadata = make_metadata("session-1");
        let entries = vec![create_user_entry("e1", "DATABASE migration")];

        sessions.insert("session-1".to_string(), (metadata, entries));

        let results = search
            .search(
                &sessions,
                "database",
                SessionSearchOptions {
                    limit: None,
                    include_tool_calls: false,
                    include_custom_entries: false,
                },
            )
            .await;

        assert_eq!(results.len(), 1);
    }
}
