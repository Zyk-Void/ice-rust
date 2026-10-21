//! Autocomplete support for TUI.
//!
//! Provides autocomplete suggestions for file paths, commands, and custom items.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::fuzzy::fuzzy_filter;

/// Path delimiters for parsing.
const PATH_DELIMITERS: &[char] = &[' ', '\t', '\n', '\r', '"', '\'', '='];

/// Greatest char-boundary byte index in `s` that is `<= idx` (clamped to
/// `s.len()`). Callers receive a `cursor` that may be a *character* count or a
/// byte offset handed back from the editor; either way, slicing
/// `&input[..cursor]` panics if `cursor` lands inside a multibyte char. Snap to
/// the nearest preceding boundary so the slice is always sound.
///
/// A position that IS a char boundary (including the END of the string) is
/// kept as-is — the old `take_while(b <= idx)` returned the START of the char
/// at `idx`, so `end: cursor` came out one char short and Tab-accept kept the
/// input's last char: typing "/mode" + Tab produced "/modele" instead of
/// "/model".
fn snap_cursor(s: &str, idx: usize) -> usize {
    let idx = idx.min(s.len());
    if s.is_char_boundary(idx) {
        return idx;
    }
    s.char_indices()
        .take_while(|(b, _)| *b < idx)
        .last()
        .map(|(b, _)| b)
        .unwrap_or(0)
}

/// A path token and the exact byte range replaced by completion. Quoted
/// paths keep spaces in the query and consume an existing closing quote.
struct PathToken<'a> {
    query: &'a str,
    start: usize,
    end: usize,
    quoted: bool,
    at: bool,
}

fn path_token(input: &str, cursor: usize) -> PathToken<'_> {
    let cursor = snap_cursor(input, cursor);
    let before = &input[..cursor];
    let mut quote = None;
    for (index, ch) in before.char_indices() {
        if ch == '"' {
            quote = if quote.is_some() { None } else { Some(index) };
        }
    }
    if let Some(start) = quote {
        let at = input[..start].ends_with('@')
            && (start == 1
                || input[..start - 1]
                    .chars()
                    .last()
                    .map_or(true, |ch| PATH_DELIMITERS.contains(&ch)));
        return PathToken {
            query: &input[start + 1..cursor],
            start,
            end: cursor + usize::from(input[cursor..].starts_with('"')),
            quoted: true,
            at,
        };
    }
    let raw_start = before.rfind(PATH_DELIMITERS).map_or(0, |i| i + 1);
    let at = before[raw_start..].starts_with('@');
    let start = raw_start + usize::from(at);
    PathToken {
        query: &input[start..cursor],
        start,
        end: cursor,
        quoted: false,
        at,
    }
}

fn path_items_for_token(
    items: Vec<AutocompleteItem>,
    token: &PathToken<'_>,
) -> Vec<AutocompleteItem> {
    items
        .into_iter()
        .map(|mut item| {
            if token.quoted || item.text.chars().any(char::is_whitespace) {
                item.text = format!("\"{}\"", item.text);
            }
            item
        })
        .collect()
}

/// An autocomplete item.
#[derive(Debug, Clone)]
pub struct AutocompleteItem {
    /// Display text.
    pub text: String,
    /// Display label (optional, defaults to text).
    pub label: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Whether this is a directory.
    pub is_directory: bool,
    /// Whether to insert a space after this item.
    pub insert_space: bool,
}

impl AutocompleteItem {
    /// Create a new autocomplete item.
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            label: None,
            description: None,
            is_directory: false,
            insert_space: true,
        }
    }

    /// Add a label.
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    /// Add a description.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    /// Mark as directory.
    pub fn as_directory(mut self) -> Self {
        self.is_directory = true;
        self
    }

    /// Set whether to insert space after.
    pub fn with_insert_space(mut self, insert_space: bool) -> Self {
        self.insert_space = insert_space;
        self
    }

    /// Get the display text.
    pub fn display_text(&self) -> &str {
        self.label.as_ref().unwrap_or(&self.text)
    }
}

/// Autocomplete suggestions result.
#[derive(Debug, Clone)]
pub struct AutocompleteSuggestions {
    /// List of suggestions.
    pub items: Vec<AutocompleteItem>,
    /// Start position in the input.
    pub start: usize,
    /// End position in the input.
    pub end: usize,
    /// Whether this is a path completion.
    pub is_path: bool,
}

/// Autocomplete provider trait.
pub trait AutocompleteProvider: Send + Sync {
    /// Get suggestions for the given input.
    fn get_suggestions(&self, input: &str, cursor: usize) -> Option<AutocompleteSuggestions>;

    /// Get the name of this provider.
    fn name(&self) -> &str;
}

/// Slash command for command completion.
#[derive(Debug, Clone)]
pub struct SlashCommand {
    /// Command name (including the slash).
    pub name: String,
    /// Description.
    pub description: String,
}

/// File path autocomplete provider.
pub struct FilePathAutocompleteProvider {
    root_path: Option<PathBuf>,
    max_results: usize,
}

impl FilePathAutocompleteProvider {
    /// Create a new file path provider.
    pub fn new() -> Self {
        Self {
            root_path: None,
            max_results: 50,
        }
    }

    /// Create a provider with a root path.
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root_path: Some(root),
            max_results: 50,
        }
    }

    /// Set max results.
    pub fn with_max_results(mut self, max: usize) -> Self {
        self.max_results = max;
        self
    }

    /// Get completions for a path prefix.
    fn get_path_completions(&self, prefix: &str) -> Vec<AutocompleteItem> {
        let mut items = Vec::new();

        // Determine the directory to search
        let normalized = prefix.replace('\\', "/");
        let (display_base, partial) = match normalized.rfind('/') {
            Some(slash) => (&normalized[..=slash], &normalized[slash + 1..]),
            None => ("", normalized.as_str()),
        };
        let dir = if let Some(rest) = display_base.strip_prefix("~/") {
            let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
            else {
                return items;
            };
            PathBuf::from(home).join(rest)
        } else {
            self.root_path
                .as_deref()
                .unwrap_or(Path::new("."))
                .join(display_base)
        };

        // Filter the whole directory before limiting: `read_dir` order is
        // unspecified, so cutting first dropped matches in large directories.
        let partial = partial.to_lowercase();
        let mut matches: Vec<(bool, String)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if !partial.is_empty() && !name.to_lowercase().starts_with(&partial) {
                    return None;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                Some((is_dir, name))
            })
            .collect();
        // Directories first, then case-insensitive name, so results are stable.
        matches.sort_by(|(a_dir, a), (b_dir, b)| {
            b_dir
                .cmp(a_dir)
                .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
                .then_with(|| a.cmp(b))
        });
        for (is_dir, name) in matches.into_iter().take(self.max_results) {
            let label = if is_dir { format!("{name}/") } else { name };
            let text = format!("{display_base}{label}");
            let item = AutocompleteItem::new(&text)
                .with_label(&label)
                .with_insert_space(!is_dir);
            items.push(if is_dir { item.as_directory() } else { item });
        }

        items
    }
}

impl Default for FilePathAutocompleteProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompleteProvider for FilePathAutocompleteProvider {
    fn get_suggestions(&self, input: &str, cursor: usize) -> Option<AutocompleteSuggestions> {
        let token = path_token(input, cursor);
        if token.query.is_empty() && !token.at && !token.quoted {
            return None;
        }

        let items = path_items_for_token(self.get_path_completions(token.query), &token);
        if items.is_empty() {
            return None;
        }

        Some(AutocompleteSuggestions {
            items,
            start: token.start,
            end: token.end,
            is_path: true,
        })
    }

    fn name(&self) -> &str {
        "filepath"
    }
}

/// A recursive file search: `(base directory, query)` to entries relative to
/// the base, `/`-separated, each with whether it is a directory.
pub type FileSearchFn = dyn Fn(&Path, &str) -> Vec<(String, bool)> + Send + Sync;

/// Recursive `@` attachment search (ice `getFuzzyFileSuggestions`): typing
/// `@parser` finds `crates/x/src/parser.rs` anywhere under the root.
///
/// `get_suggestions` runs on the input thread for every key press, so it
/// never blocks. It returns the ranked results cached for the current query,
/// or `None` while a background search runs (so a later provider, such as
/// the one-directory [`FilePathAutocompleteProvider`], answers meanwhile).
/// One search runs at a time; when it finishes, the newest query typed in the
/// meantime is searched next, results for older queries are dropped, and
/// `notify` is called so the host can refresh the menu.
pub struct RecursiveFileAutocompleteProvider {
    root: PathBuf,
    search: Arc<FileSearchFn>,
    notify: Arc<dyn Fn() + Send + Sync>,
    state: Arc<Mutex<RecursiveSearchState>>,
}

#[derive(Default)]
struct RecursiveSearchState {
    /// The query the user is currently on.
    wanted: Option<String>,
    generation: u64,
    /// Whether a background search is running.
    running: bool,
    /// The last finished query and its ranked items.
    results: Option<(String, Vec<AutocompleteItem>)>,
}

/// Entries fetched from the search before ranking, and items shown (ice).
const RECURSIVE_SEARCH_LIMIT: usize = 100;
const RECURSIVE_SHOWN_LIMIT: usize = 20;
/// Longest an `fd` search may run before it is abandoned.
const FD_SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

impl RecursiveFileAutocompleteProvider {
    /// A provider that searches with `fd` (or Debian's `fdfind`), which
    /// honours `.gitignore`. `None` when neither is on `PATH`; ice also has no
    /// recursive search without `fd`.
    pub fn with_fd(root: PathBuf, notify: Arc<dyn Fn() + Send + Sync>) -> Option<Self> {
        let fd = ["fd", "fdfind"]
            .iter()
            .find_map(|name| find_on_path(name))?;
        let search: Arc<FileSearchFn> =
            Arc::new(move |base: &Path, query: &str| fd_search(&fd, base, query));
        Some(Self::with_search(root, search, notify))
    }

    /// A provider backed by any search function (used by tests).
    pub fn with_search(
        root: PathBuf,
        search: Arc<FileSearchFn>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            root,
            search,
            notify,
            state: Arc::new(Mutex::new(RecursiveSearchState::default())),
        }
    }

    /// Search for `query` on a background thread, then for whatever query is
    /// wanted by the time it finishes, until the results match it.
    fn spawn_search(&self, query: String, generation: u64) {
        let root = self.root.clone();
        let search = self.search.clone();
        let notify = self.notify.clone();
        let state = self.state.clone();
        std::thread::spawn(move || {
            let mut query = query;
            let mut generation = generation;
            loop {
                let items = rank_recursive_entries(&root, &query, search.as_ref());
                let Ok(mut st) = state.lock() else { return };
                if st.generation == generation && st.wanted.as_deref() == Some(query.as_str()) {
                    st.results = Some((query.clone(), items));
                }
                match st.wanted.clone() {
                    Some(next) if st.generation != generation => {
                        query = next;
                        generation = st.generation;
                    }
                    _ => {
                        st.running = false;
                        break;
                    }
                }
            }
            notify();
        });
    }
}

impl AutocompleteProvider for RecursiveFileAutocompleteProvider {
    fn get_suggestions(&self, input: &str, cursor: usize) -> Option<AutocompleteSuggestions> {
        let token = path_token(input, cursor);
        let mut st = self.state.lock().ok()?;
        if !token.at {
            if st.wanted.take().is_some() {
                st.generation = st.generation.wrapping_add(1);
            }
            return None;
        }
        let query = token.query;
        if st.wanted.as_deref() != Some(query) {
            st.wanted = Some(query.to_string());
            st.generation = st.generation.wrapping_add(1);
        }
        if let Some((done, items)) = &st.results {
            if done == query {
                return (!items.is_empty()).then(|| AutocompleteSuggestions {
                    items: path_items_for_token(items.clone(), &token),
                    start: token.start,
                    end: token.end,
                    is_path: true,
                });
            }
        }
        if !st.running {
            st.running = true;
            let generation = st.generation;
            drop(st);
            self.spawn_search(query.to_string(), generation);
        }
        None
    }

    fn name(&self) -> &str {
        "recursive-file"
    }
}

/// Run `search` for `raw_query` and rank the entries (ice
/// `getFuzzyFileSuggestions` + `scoreEntry`). A query with a `/` searches
/// inside that directory for the part after the last `/`.
fn rank_recursive_entries(
    root: &Path,
    raw_query: &str,
    search: &FileSearchFn,
) -> Vec<AutocompleteItem> {
    let normalized = raw_query.replace('\\', "/");
    let raw_query = normalized.as_str();
    let (base, query, display_base) = match raw_query.rfind('/') {
        Some(slash) => {
            let display_base = &raw_query[..=slash];
            let base = if let Some(rest) = display_base.strip_prefix("~/") {
                match std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                    Some(home) => PathBuf::from(home).join(rest),
                    None => return Vec::new(),
                }
            } else {
                root.join(display_base)
            };
            if base.is_dir() {
                (base, &raw_query[slash + 1..], display_base)
            } else {
                // A directory fragment may be in the middle of a deeper path,
                // rather than relative to the configured root.
                (root.to_path_buf(), raw_query, "")
            }
        }
        None => (root.to_path_buf(), raw_query, ""),
    };
    let lower_query = query.to_lowercase();
    let score = |path: &str, is_dir: bool| -> u32 {
        if query.is_empty() {
            return 1;
        }
        let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
        let base = if name == lower_query {
            100
        } else if name.starts_with(&lower_query) {
            80
        } else if name.contains(&lower_query) {
            50
        } else if path.to_lowercase().contains(&lower_query) {
            30
        } else {
            0
        };
        if is_dir && base > 0 {
            base + 10
        } else {
            base
        }
    };
    let mut scored: Vec<(u32, String, bool)> = search(&base, query)
        .into_iter()
        .take(RECURSIVE_SEARCH_LIMIT)
        .filter_map(|(path, is_dir)| {
            let path = path.trim_end_matches('/').to_string();
            let s = score(&path, is_dir);
            (s > 0).then_some((s, path, is_dir))
        })
        .collect();
    // Stable: equal scores keep the search order.
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(RECURSIVE_SHOWN_LIMIT)
        .map(|(_, path, is_dir)| {
            let display = format!("{display_base}{path}");
            let name = path.rsplit('/').next().unwrap_or(&path);
            let (text, label) = if is_dir {
                (format!("{display}/"), format!("{name}/"))
            } else {
                (display.clone(), name.to_string())
            };
            let item = AutocompleteItem::new(&text)
                .with_label(&label)
                .with_description(&display)
                .with_insert_space(!is_dir);
            if is_dir {
                item.as_directory()
            } else {
                item
            }
        })
        .collect()
}

/// `fd` over `base` with ice's arguments; entries come back relative to the
/// base, directories with a trailing `/`. `--fixed-strings` keeps a typed
/// `.` or `(` literal. Errors and non-zero exits give no entries.
fn fd_search(fd: &Path, base: &Path, query: &str) -> Vec<(String, bool)> {
    let mut command = std::process::Command::new(fd);
    command
        .arg("--base-directory")
        .arg(base)
        .args(["--max-results", &RECURSIVE_SEARCH_LIMIT.to_string()])
        .args(["--type", "f", "--type", "d", "--follow", "--hidden"])
        // Literal, case-insensitive matching, the same as the ranking (fd's
        // default smart case would drop `parser.rs` for `@Parser`).
        .args(["--fixed-strings", "--ignore-case"])
        .arg("--print0")
        .args([
            "--exclude",
            ".git",
            "--exclude",
            ".git/*",
            "--exclude",
            ".git/**",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if !query.is_empty() {
        if query.contains('/') {
            command.arg("--full-path");
        }
        command.arg("--").arg(query);
    }
    let Ok(mut child) = command.spawn() else {
        return Vec::new();
    };
    // Drain concurrently: 100 long paths can exceed the OS pipe buffer.
    // Bound memory independently of the entry limit and bound process time.
    let Some(pipe) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Vec::new();
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut limited = std::io::Read::take(pipe, 1024 * 1024);
        let _ = std::io::Read::read_to_end(&mut limited, &mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + FD_SEARCH_TIMEOUT;
    let mut success = false;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                success = true;
                break;
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let stdout = reader.join().unwrap_or_default();
    if !success {
        return Vec::new();
    }
    String::from_utf8_lossy(&stdout)
        .split('\0')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let line = line.replace('\\', "/");
            let line = line.strip_prefix("./").unwrap_or(&line).to_string();
            let is_dir = line.ends_with('/');
            (line, is_dir)
        })
        .filter(|(line, _)| {
            let path = line.trim_end_matches('/');
            path != ".git" && !path.starts_with(".git/") && !path.contains("/.git/")
        })
        .collect()
}

/// First executable named `name` on `PATH` (with `.exe` on Windows).
fn find_on_path(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Slash command autocomplete provider.
pub struct SlashCommandAutocompleteProvider {
    commands: Vec<SlashCommand>,
}

impl SlashCommandAutocompleteProvider {
    /// Create a new slash command provider.
    pub fn new(commands: Vec<SlashCommand>) -> Self {
        Self { commands }
    }

    /// Create with default commands plus a set of dynamically-discovered slash
    /// commands. The discovered commands (e.g. prompt-template names exposed as
    /// `/expand`-style `/`-prefixed entries) are appended to the built-in set so
    /// the built-in names win on a fuzzy tie and the discovered names appear as
    /// additional suggestions. Mirrors pi's behavior where prompt-template
    /// invocations (`/<name>`) surface alongside built-in slash commands in
    /// `/`-autocomplete (`agent-session.ts:1124` + `expandPromptTemplate`).
    pub fn with_commands(discovered: Vec<SlashCommand>) -> Self {
        let mut commands = Self::with_default_commands().commands;
        commands.extend(discovered);
        Self { commands }
    }

    /// Create with default commands.
    pub fn with_default_commands() -> Self {
        Self::new(vec![
            SlashCommand {
                name: "/help".to_string(),
                description: "Show help information".to_string(),
            },
            SlashCommand {
                name: "/clear".to_string(),
                description: "Clear the conversation".to_string(),
            },
            SlashCommand {
                name: "/exit".to_string(),
                description: "Exit the application".to_string(),
            },
            SlashCommand {
                name: "/model".to_string(),
                description: "Change the model".to_string(),
            },
            SlashCommand {
                name: "/context".to_string(),
                description: "List discovered context files, skills, and prompt templates"
                    .to_string(),
            },
        ])
    }

    /// Add a command.
    pub fn add_command(&mut self, command: SlashCommand) {
        self.commands.push(command);
    }
}

impl AutocompleteProvider for SlashCommandAutocompleteProvider {
    fn get_suggestions(&self, input: &str, cursor: usize) -> Option<AutocompleteSuggestions> {
        let cursor = snap_cursor(input, cursor);
        let before_cursor = &input[..cursor];

        // Check if we're at the start of a slash command
        if !before_cursor.starts_with('/') {
            return None;
        }

        let query = &before_cursor[1..];
        let items: Vec<AutocompleteItem> = fuzzy_filter(&self.commands, query, |cmd| &cmd.name)
            .into_iter()
            .map(|cmd| {
                AutocompleteItem::new(&cmd.name)
                    .with_description(&cmd.description)
                    .with_insert_space(false)
            })
            .collect();

        if items.is_empty() {
            return None;
        }

        Some(AutocompleteSuggestions {
            items,
            start: 0,
            end: cursor,
            is_path: false,
        })
    }

    fn name(&self) -> &str {
        "slash_command"
    }
}

/// Combined autocomplete provider that chains multiple providers.
pub struct CombinedAutocompleteProvider {
    providers: Vec<Arc<dyn AutocompleteProvider>>,
}

impl CombinedAutocompleteProvider {
    /// Create a new combined provider.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Add a provider.
    pub fn add_provider(&mut self, provider: Arc<dyn AutocompleteProvider>) {
        self.providers.push(provider);
    }

    /// Create with default providers.
    pub fn with_defaults() -> Self {
        let mut combined = Self::new();
        combined.add_provider(Arc::new(
            SlashCommandAutocompleteProvider::with_default_commands(),
        ));
        combined.add_provider(Arc::new(FilePathAutocompleteProvider::new()));
        combined
    }
}

impl Default for CombinedAutocompleteProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompleteProvider for CombinedAutocompleteProvider {
    fn get_suggestions(&self, input: &str, cursor: usize) -> Option<AutocompleteSuggestions> {
        for provider in &self.providers {
            if let Some(suggestions) = provider.get_suggestions(input, cursor) {
                return Some(suggestions);
            }
        }
        None
    }

    fn name(&self) -> &str {
        "combined"
    }
}

/// Global autocomplete manager.
pub struct AutocompleteManager {
    provider: Mutex<Option<Arc<dyn AutocompleteProvider>>>,
}

impl AutocompleteManager {
    /// Create a new manager.
    pub fn new() -> Self {
        Self {
            provider: Mutex::new(None),
        }
    }

    /// Set the provider.
    pub fn set_provider(&self, provider: Arc<dyn AutocompleteProvider>) {
        if let Ok(mut p) = self.provider.lock() {
            *p = Some(provider);
        }
    }

    /// Get suggestions.
    pub fn get_suggestions(&self, input: &str, cursor: usize) -> Option<AutocompleteSuggestions> {
        self.provider
            .lock()
            .ok()?
            .as_ref()?
            .get_suggestions(input, cursor)
    }
}

impl Default for AutocompleteManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider over a fixed entry list, and a receiver that fires when a
    /// background search finishes. The search records each query it ran.
    fn recursive_provider(
        root: PathBuf,
        entries: &'static [(&'static str, bool)],
    ) -> (
        RecursiveFileAutocompleteProvider,
        std::sync::mpsc::Receiver<()>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let queries = Arc::new(Mutex::new(Vec::new()));
        let seen = queries.clone();
        let search: Arc<FileSearchFn> = Arc::new(move |_base: &Path, query: &str| {
            seen.lock().unwrap().push(query.to_string());
            entries
                .iter()
                .filter(|(path, _)| path.to_lowercase().contains(&query.to_lowercase()))
                .map(|(path, is_dir)| (path.to_string(), *is_dir))
                .collect()
        });
        let notify = Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        });
        (
            RecursiveFileAutocompleteProvider::with_search(root, search, notify),
            rx,
            queries,
        )
    }

    fn texts(suggestions: &AutocompleteSuggestions) -> Vec<&str> {
        suggestions.items.iter().map(|i| i.text.as_str()).collect()
    }

    /// ice `getFuzzyFileSuggestions`: `@query` searches the whole tree and
    /// ranks exact name, then prefix, then substring, then path matches, with
    /// directories ahead on ties. The first call never blocks.
    #[test]
    fn recursive_at_search_ranks_matches_across_the_tree() {
        const ENTRIES: &[(&str, bool)] = &[
            ("docs/parser-notes.md", false),
            ("crates/x/src/parser.rs", false),
            ("crates/parser/", true),
            ("crates/x/src/my_parser.rs", false),
            ("parser/tests/a.rs", false),
        ];
        let (provider, ready, _) = recursive_provider(PathBuf::from("/repo"), ENTRIES);
        assert!(provider.get_suggestions("see @parser", 11).is_none());
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let found = provider.get_suggestions("see @parser", 11).unwrap();
        assert_eq!(
            texts(&found),
            vec![
                "crates/parser/",
                "docs/parser-notes.md",
                "crates/x/src/parser.rs",
                "crates/x/src/my_parser.rs",
                "parser/tests/a.rs",
            ]
        );
        assert_eq!((found.start, found.end), (5, 11));
        assert_eq!(found.items[0].label.as_deref(), Some("parser/"));
        assert!(found.items[0].is_directory && !found.items[0].insert_space);
        assert!(provider.get_suggestions("no at here", 10).is_none());
    }

    /// Queries typed while a search runs are not lost: the newest one is
    /// searched next and older results never answer a newer query.
    #[test]
    fn recursive_at_search_follows_the_newest_query() {
        const ENTRIES: &[(&str, bool)] = &[("src/alpha.rs", false), ("src/beta.rs", false)];
        let (provider, ready, queries) = recursive_provider(PathBuf::from("/repo"), ENTRIES);
        assert!(provider.get_suggestions("@al", 3).is_none());
        assert!(provider.get_suggestions("@be", 3).is_none());
        // The "al" search may finish first; "be" must still be searched.
        let found = loop {
            ready
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            if let Some(found) = provider.get_suggestions("@be", 3) {
                break found;
            }
        };
        assert_eq!(texts(&found), vec!["src/beta.rs"]);
        assert_eq!(
            queries.lock().unwrap().last().map(String::as_str),
            Some("be")
        );
    }

    /// A query with a `/` searches inside that directory and keeps the
    /// typed directory in the completion.
    #[test]
    fn recursive_at_search_scopes_to_a_typed_directory() {
        let dir = std::env::temp_dir().join(format!("ice-tui-at-scope-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("crates")).unwrap();
        const ENTRIES: &[(&str, bool)] = &[("x/lib.rs", false)];
        let (provider, ready, queries) = recursive_provider(dir.clone(), ENTRIES);
        assert!(provider.get_suggestions("@crates/lib", 11).is_none());
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let found = provider.get_suggestions("@crates/lib", 11).unwrap();
        assert_eq!(texts(&found), vec!["crates/x/lib.rs"]);
        assert_eq!(queries.lock().unwrap().as_slice(), ["lib"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recursive_at_search_handles_empty_extensions_and_deep_path_fragments() {
        const ENTRIES: &[(&str, bool)] = &[
            ("packages/tui/src/autocomplete.ts", false),
            ("packages/tui/src/autocomplete.test.ts", false),
            ("src/components/", true),
            ("src/components/Button.tsx", false),
        ];
        let (provider, ready, _) = recursive_provider(PathBuf::from("/missing-repo"), ENTRIES);
        for (input, expected) in [
            (
                "@",
                ENTRIES.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
            ),
            ("@AUTOCOMPLETE.TS", vec!["packages/tui/src/autocomplete.ts"]),
            (
                "@tui/src/auto",
                vec![
                    "packages/tui/src/autocomplete.ts",
                    "packages/tui/src/autocomplete.test.ts",
                ],
            ),
            ("@components/", vec!["src/components/Button.tsx"]),
        ] {
            assert!(provider.get_suggestions(input, input.len()).is_none());
            ready
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let found = provider.get_suggestions(input, input.len()).unwrap();
            assert_eq!(texts(&found), expected, "query {input}");
        }
    }

    #[test]
    fn recursive_at_search_quotes_spaces_and_replaces_only_the_token() {
        const ENTRIES: &[(&str, bool)] = &[("my folder/test file.txt", false)];
        let (provider, ready, _) = recursive_provider(PathBuf::from("/missing-repo"), ENTRIES);
        let input = "查看 @\"my folder/te\" suffix";
        let cursor = "查看 @\"my folder/te".len();
        assert!(provider.get_suggestions(input, cursor).is_none());
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let found = provider.get_suggestions(input, cursor).unwrap();
        assert_eq!(texts(&found), vec!["\"my folder/test file.txt\""]);
        assert_eq!(&input[..found.start], "查看 @");
        assert_eq!(&input[found.end..], " suffix");
        assert!(!found.items[0].is_directory && found.items[0].insert_space);
    }

    #[test]
    fn recursive_at_search_limits_both_backend_entries_and_shown_items() {
        let search = |_base: &Path, _query: &str| {
            (0..150)
                .map(|index| (format!("entry{index:03}"), false))
                .collect()
        };
        let items = rank_recursive_entries(Path::new("/repo"), "", &search);
        assert_eq!(items.len(), RECURSIVE_SHOWN_LIMIT);
        assert_eq!(items.last().unwrap().text, "entry019");
        // A high-ranking entry beyond the backend cap must not be included.
        assert!(rank_recursive_entries(Path::new("/repo"), "entry149", &search).is_empty());
    }

    /// The real `fd` backend, when installed: finds a nested file and skips
    /// `.git`. Passes without running when `fd` is not on `PATH`.
    #[test]
    fn recursive_at_search_with_fd_when_available() {
        let Some(fd) = ["fd", "fdfind"].iter().find_map(|n| find_on_path(n)) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("ice-tui-at-fd-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("a/b/needle.txt"), "").unwrap();
        std::fs::write(dir.join(".git/needle"), "").unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored-needle.txt\n").unwrap();
        std::fs::write(dir.join("ignored-needle.txt"), "").unwrap();
        let entries = fd_search(&fd, &dir, "needle");
        assert_eq!(entries, vec![("a/b/needle.txt".to_string(), false)]);
        // Case-insensitive, like the ranking.
        assert_eq!(fd_search(&fd, &dir, "NEEDLE"), entries);
        assert_eq!(fd_search(&fd, &dir, "a/b/needle"), entries);
        std::fs::create_dir_all(dir.join(".settings")).unwrap();
        std::fs::write(dir.join(".settings/hidden-file.txt"), "").unwrap();
        assert_eq!(
            fd_search(&fd, &dir, "hidden-file"),
            vec![(".settings/hidden-file.txt".into(), false)]
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("a"), dir.join("linked")).unwrap();
            assert!(fd_search(&fd, &dir, "needle")
                .iter()
                .any(|(path, _)| path == "linked/b/needle.txt"));
            let long_dir = dir
                .join("a".repeat(180))
                .join("b".repeat(180))
                .join("c".repeat(180))
                .join("d".repeat(180))
                .join("e".repeat(180));
            std::fs::create_dir_all(&long_dir).unwrap();
            for index in 0..100 {
                std::fs::write(long_dir.join(format!("bulk-{index:03}.txt")), "").unwrap();
            }
            // This output is larger than a pipe buffer: waiting for exit
            // before draining stdout used to time out and lose every result.
            assert_eq!(fd_search(&fd, &dir, "bulk-").len(), 100);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: the 50-entry limit ran before the prefix filter, so in a
    /// directory with more entries the typed file was often missing.
    #[test]
    fn path_completion_filters_before_limiting_and_sorts() {
        let dir = std::env::temp_dir().join(format!("ice-tui-path-ac-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..80 {
            std::fs::write(dir.join(format!("file{i:02}.txt")), "").unwrap();
        }
        std::fs::write(dir.join("zz-target.rs"), "").unwrap();
        std::fs::create_dir(dir.join("Zdir")).unwrap();
        std::fs::write(dir.join("zebra.rs"), "").unwrap();
        let provider = FilePathAutocompleteProvider::with_root(dir.clone());

        let found = provider.get_suggestions("open zz", 7).unwrap();
        let texts: Vec<_> = found.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, vec!["zz-target.rs"]);

        let found = provider.get_suggestions("@z", 2).unwrap();
        let texts: Vec<_> = found.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, vec!["Zdir/", "zebra.rs", "zz-target.rs"]);

        let all = provider.get_suggestions("@", 1).unwrap();
        assert_eq!(all.items.len(), 50);
        assert_eq!(all.items[0].text, "Zdir/");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_completion_keeps_quoted_directory_and_utf8_replacement_range() {
        let dir = std::env::temp_dir().join(format!("ice-tui-quoted-path-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("my folder")).unwrap();
        std::fs::write(dir.join("my folder/test file.txt"), "").unwrap();
        let provider = FilePathAutocompleteProvider::with_root(dir.clone());
        let input = "查看 @\"my folder/te\" suffix";
        let cursor = "查看 @\"my folder/te".len();
        let found = provider.get_suggestions(input, cursor).unwrap();
        assert_eq!(texts(&found), vec!["\"my folder/test file.txt\""]);
        assert_eq!(&input[..found.start], "查看 @");
        assert_eq!(&input[found.end..], " suffix");
        assert!(!found.items[0].is_directory);
        let found = provider.get_suggestions("@my", 3).unwrap();
        assert_eq!(texts(&found), vec!["\"my folder/\""]);
        assert!(found.items[0].is_directory && !found.items[0].insert_space);
        let found = provider.get_suggestions("@\"my folder/", 12).unwrap();
        assert_eq!(texts(&found), vec!["\"my folder/test file.txt\""]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn snap_cursor_keeps_boundaries_including_end() {
        // Regression: the caret at the END of the input (what Tab-accept sees)
        // snapped one char short, so `end` excluded the last char and accept
        // kept it: "/mode" + Tab became "/modele" instead of "/model".
        assert_eq!(snap_cursor("/mode", 5), 5);
        assert_eq!(snap_cursor("/mo", 3), 3);
        assert_eq!(snap_cursor("/model", 6), 6);
        assert_eq!(snap_cursor("abc", 3), 3);
        // Interior boundary + mid-multibyte snap down.
        assert_eq!(snap_cursor("/mode", 2), 2);
        assert_eq!(snap_cursor("你", 1), 0);
        assert_eq!(snap_cursor("你", 3), 3);
        // Out-of-range clamps to the end.
        assert_eq!(snap_cursor("/mode", 99), 5);
    }
    #[test]
    fn test_slash_command_provider() {
        let provider = SlashCommandAutocompleteProvider::with_default_commands();
        let suggestions = provider.get_suggestions("/he", 3);

        assert!(suggestions.is_some());
        let s = suggestions.unwrap();
        assert!(!s.items.is_empty());
        assert!(s.items[0].text.starts_with("/help"));
    }

    #[test]
    fn test_autocomplete_item() {
        let item = AutocompleteItem::new("test.txt")
            .with_description("A test file")
            .as_directory();

        assert_eq!(item.text, "test.txt");
        assert!(item.is_directory);
    }
}
