//! The `mcp` gateway tool: status, list, search, describe, instructions,
//! connect, and single calls to any MCP tool, direct or not. Ports
//! pi-mcp-adapter `proxy-modes.ts`, `search-ranking.ts` and the name
//! resolution in `namespace-tools.ts`.
use super::*;
use elicitation::clean;

pub(super) const NAME: &str = "mcp";
const PAGE: usize = 12;
const SUGGESTIONS: usize = 5;

/// One callable entry, live or from the metadata cache.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// The prefixed name, as a direct tool would be registered.
    pub name: String,
    pub remote: String,
    pub description: String,
    pub schema: Value,
    pub resource_uri: Option<String>,
}
impl Entry {
    pub(super) fn from_tool(tool: &McpTool) -> Self {
        Self {
            name: tool.schema.name.clone(),
            remote: tool.remote_name.clone(),
            description: tool.schema.description.clone(),
            schema: serde_json::to_value(&tool.schema.parameters).unwrap_or(Value::Null),
            resource_uri: tool.resource_uri.clone(),
        }
    }
    /// Rebuilds names the same way discovery does, so cached and live entries match.
    pub(super) fn from_cache(
        server: &str,
        cached: &cache::CachedServer,
        config: &ServerConfig,
    ) -> Vec<Self> {
        let mut entries: Vec<_> = cached
            .tools
            .iter()
            .filter_map(|item| {
                let remote = item.get("name")?.as_str()?;
                Some(Self {
                    name: format!("{server}_{remote}"),
                    remote: remote.into(),
                    description: item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("MCP tool")
                        .into(),
                    schema: item.get("inputSchema").cloned().unwrap_or(Value::Null),
                    resource_uri: None,
                })
            })
            .collect();
        if config.expose_resources {
            let mut used = HashSet::new();
            entries.extend(cached.resources.iter().filter_map(|item| {
                let remote = unique_resource_name(&mut used, item.get("name")?.as_str()?);
                Some(Self {
                    name: format!("{server}_{remote}"),
                    description: item
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("Read MCP resource")
                        .chars()
                        .filter(|c| !c.is_control())
                        .collect(),
                    remote,
                    schema: json!({"type":"object","properties":{}}),
                    resource_uri: Some(item.get("uri")?.as_str()?.into()),
                })
            }));
        }
        entries
    }
}

pub(crate) struct Server {
    pub name: String,
    pub status: ServerStatus,
    /// `entries` and `instructions` come from the metadata cache.
    pub cached: bool,
    pub entries: Vec<Entry>,
    pub instructions: Option<String>,
    pub config: ServerConfig,
}

pub(super) struct ProxyTool {
    manager: std::sync::Weak<manager::Inner>,
    schema: Tool,
}
impl ProxyTool {
    pub(super) fn new(manager: std::sync::Weak<manager::Inner>) -> Self {
        let parameters = json!({
            "type": "object",
            "properties": {
                "tool": {"type":"string","description":"MCP tool to call, e.g. \"github_search_issues\""},
                "args": {"description":"Arguments for `tool`: an object, or a JSON string"},
                "server": {"type":"string","description":"List this server's tools, or the server that owns `tool`"},
                "search": {"type":"string","description":"Space-separated words to find tools by name, server and description"},
                "describe": {"type":"string","description":"Show one tool's description and parameters"},
                "instructions": {"type":"string","description":"Show a server's full instructions"},
                "connect": {"type":"string","description":"Connect or reconnect a server to refresh its catalog"},
                "limit": {"type":"integer","description":"Search page size (default 12)"},
                "offset": {"type":"integer","description":"Search results to skip"}
            },
            "additionalProperties": false
        });
        Self {
            manager,
            schema: Tool {
                name: NAME.into(),
                description: "MCP gateway. With no arguments, shows server status. `server` lists a server's tools, `search` finds tools, `describe` shows a tool's parameters, `instructions` shows a server's guidance, `connect` refreshes a server, and `tool` with `args` calls one MCP tool, including tools that are not registered directly.".into(),
                parameters: serde_json::from_value(parameters).expect("static schema"),
                constrained_sampling: None,
            },
        }
    }
    fn manager(&self) -> Result<McpManager, AgentError> {
        self.manager
            .upgrade()
            .map(McpManager)
            .ok_or_else(|| AgentError::Tool("MCP session ended".into()))
    }
}

fn text(text: String, details: Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![TextContentOrImage::text(text)],
        details,
        ..Default::default()
    }
}

#[async_trait]
impl AgentTool for ProxyTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "MCP"
    }
    async fn execute(
        &self,
        call_id: &str,
        params: Value,
        signal: CancellationToken,
        on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let manager = self.manager()?;
        let field = |key: &str| params.get(key).and_then(Value::as_str);
        if let Some(server) = field("connect") {
            manager
                .reconnect(Some(server))
                .await
                .map_err(AgentError::Tool)?;
            return Ok(status(&manager.proxy_view()));
        }
        let servers = manager.proxy_view();
        if let Some(tool) = field("tool") {
            return call(
                &manager,
                &servers,
                tool,
                field("server"),
                &params,
                call_id,
                signal,
                on_update,
            )
            .await;
        }
        if let Some(name) = field("describe") {
            return Ok(describe(&servers, name));
        }
        if let Some(name) = field("instructions") {
            return Ok(instructions(&servers, name));
        }
        if let Some(query) = field("search") {
            let number = |key: &str| params.get(key).and_then(Value::as_u64).map(|n| n as usize);
            return Ok(search(
                &servers,
                query,
                field("server"),
                number("limit").unwrap_or(PAGE).clamp(1, 100),
                number("offset").unwrap_or(0),
            ));
        }
        if let Some(server) = field("server") {
            return Ok(list(&servers, server));
        }
        Ok(status(&servers))
    }
}

fn status(servers: &[Server]) -> AgentToolResult {
    let enabled: Vec<_> = servers
        .iter()
        .filter(|s| !matches!(s.status, ServerStatus::Disabled))
        .collect();
    let connected = enabled
        .iter()
        .filter(|s| matches!(s.status, ServerStatus::Connected { .. }))
        .count();
    let tools: usize = enabled.iter().map(|s| s.entries.len()).sum();
    let mut text = format!("MCP: {connected}/{} servers, {tools} tools", enabled.len());
    if servers.len() > enabled.len() {
        text.push_str(&format!(" ({} disabled)", servers.len() - enabled.len()));
    }
    text.push_str("\n\n");
    for server in servers {
        let name = clean(&server.name);
        text.push_str(&match &server.status {
            ServerStatus::Disabled => format!("⊘ {name} (disabled)\n"),
            ServerStatus::Connected { .. } => {
                format!("✓ {name} ({} tools)\n", server.entries.len())
            }
            ServerStatus::Failed(error) => {
                format!("✗ {name} (failed: {})\n", clean(error))
            }
            ServerStatus::Connecting if server.cached => {
                format!(
                    "○ {name} ({} tools, cached; connecting)\n",
                    server.entries.len()
                )
            }
            ServerStatus::Connecting => format!("○ {name} (connecting)\n"),
        });
    }
    if !servers.is_empty() {
        text.push_str("\nmcp({ server: \"name\" }) lists tools, mcp({ search: \"...\" }) searches");
    }
    let details = json!({
        "mode": "status",
        "servers": servers.iter().map(|s| json!({
            "name": s.name,
            "status": status_name(&s.status),
            "toolCount": s.entries.len(),
            "cached": s.cached,
        })).collect::<Vec<_>>(),
    });
    self::text(text.trim_end().into(), details)
}

fn status_name(status: &ServerStatus) -> &'static str {
    match status {
        ServerStatus::Disabled => "disabled",
        ServerStatus::Connecting => "connecting",
        ServerStatus::Connected { .. } => "connected",
        ServerStatus::Failed(_) => "failed",
    }
}

fn line(entry: &Entry, server: &Server) -> String {
    let approval =
        if entry.resource_uri.is_none() && server.config.approve_tools.requires(&entry.remote) {
            " (requires approval)"
        } else {
            ""
        };
    format!(
        "{}{approval} — {}",
        clean(&entry.name),
        truncate(&clean(&entry.description), 160)
    )
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.into(),
    }
}

fn list(servers: &[Server], name: &str) -> AgentToolResult {
    let Some(server) = servers.iter().find(|s| s.name == name) else {
        return not_found_server(name);
    };
    let mut text = format!("{} ({})", clean(&server.name), status_name(&server.status));
    if server.cached {
        text.push_str(", from cache");
    }
    if server.entries.is_empty() {
        text.push_str("\nNo tools known yet.");
    }
    for entry in &server.entries {
        text.push('\n');
        text.push_str(&line(entry, server));
    }
    if let Some(instructions) = &server.instructions {
        text.push_str(&format!(
            "\n\nInstructions: {}",
            truncate(&clean(instructions), 200)
        ));
    }
    self::text(
        text,
        json!({"mode":"list","server":server.name,"tools":server.entries.iter().map(|e| &e.name).collect::<Vec<_>>()}),
    )
}

fn not_found_server(name: &str) -> AgentToolResult {
    text(
        format!(
            "Server \"{}\" not found. Use mcp({{}}) to see available servers.",
            clean(name)
        ),
        json!({"mode":"error","error":"server_not_found","server":name}),
    )
}

fn instructions(servers: &[Server], name: &str) -> AgentToolResult {
    let Some(server) = servers.iter().find(|s| s.name == name) else {
        return not_found_server(name);
    };
    let text = match &server.instructions {
        Some(instructions) => instructions
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect(),
        None => format!("{} has no instructions.", clean(&server.name)),
    };
    self::text(text, json!({"mode":"instructions","server":server.name}))
}

/// `-` and `_` are interchangeable and case is ignored, as in PMA.
fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "_")
}

enum Found<'a> {
    One(&'a Server, &'a Entry),
    Ambiguous(Vec<String>),
    None,
}

/// Exact prefixed name, then the normalized prefixed name, then the normalized
/// remote name; each stage must be unique to resolve.
fn resolve<'a>(servers: &'a [Server], query: &str, server: Option<&str>) -> Found<'a> {
    let scope: Vec<&Server> = servers
        .iter()
        .filter(|s| server.is_none_or(|name| s.name == name))
        .collect();
    let stages: [&dyn Fn(&Entry) -> bool; 3] = [
        &|e| e.name == query,
        &|e| normalize(&e.name) == normalize(query),
        &|e| normalize(&e.remote) == normalize(query),
    ];
    for stage in stages {
        let hits: Vec<(&Server, &Entry)> = scope
            .iter()
            .flat_map(|s| s.entries.iter().filter(|e| stage(e)).map(move |e| (*s, e)))
            .collect();
        match hits.as_slice() {
            [] => continue,
            [(server, entry)] => return Found::One(server, entry),
            _ => return Found::Ambiguous(hits.iter().map(|(_, e)| e.name.clone()).collect()),
        }
    }
    Found::None
}

fn miss(mode: &str, query: &str, servers: &[Server], found: Found) -> AgentToolResult {
    match found {
        Found::Ambiguous(names) => text(
            format!(
                "\"{}\" matches several tools: {}. Use the full name or pass `server`.",
                clean(query),
                names.join(", ")
            ),
            json!({"mode":mode,"error":"ambiguous","requestedTool":query,"matches":names}),
        ),
        _ => {
            let suggestions: Vec<String> = ranked(servers, &query.replace(['_', '-'], " "), None)
                .into_iter()
                .take(SUGGESTIONS)
                .map(|(_, _, entry)| entry.name.clone())
                .collect();
            let hint = if suggestions.is_empty() {
                String::new()
            } else {
                format!(" Did you mean: {}", suggestions.join(", "))
            };
            text(
                format!(
                    "Tool \"{}\" not found. Use mcp({{ search: \"...\" }}) to search.{hint}",
                    clean(query)
                ),
                json!({"mode":mode,"error":"tool_not_found","requestedTool":query,"suggestions":suggestions}),
            )
        }
    }
}

fn describe(servers: &[Server], name: &str) -> AgentToolResult {
    let (server, entry) = match resolve(servers, name, None) {
        Found::One(server, entry) => (server, entry),
        found => return miss("describe", name, servers, found),
    };
    let mut text = line(entry, server)
        .split_once(" — ")
        .map(|(head, _)| head.to_owned())
        .unwrap_or_default();
    text.push_str(&format!("\nServer: {}", clean(&server.name)));
    match &entry.resource_uri {
        Some(uri) => text.push_str(&format!(
            "\nType: Resource (reads from {})\n\n{}\n\nNo parameters required (resource tool).",
            clean(uri),
            clean(&entry.description)
        )),
        None => text.push_str(&format!(
            "\n\n{}\n\nParameters:\n{}",
            clean(&entry.description),
            serde_json::to_string_pretty(&entry.schema).unwrap_or_default()
        )),
    }
    self::text(
        text,
        json!({"mode":"describe","server":server.name,"tool":entry.name}),
    )
}

const WEIGHT_NAME: usize = 12;
const WEIGHT_REMOTE: usize = 10;
const WEIGHT_SERVER: usize = 8;
const WEIGHT_DESCRIPTION: usize = 5;
const WEIGHT_KEYWORDS: usize = 5;
const MIN_STEM: usize = 4;

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `*` matches any run of characters; everything else is literal.
fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let [first, middle @ .., last] = parts.as_slice() else {
        return pattern == text;
    };
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

fn keywords<'a>(server: &'a Server, entry: &Entry) -> Vec<&'a str> {
    server
        .config
        .search_keywords
        .iter()
        .filter(|(pattern, _)| glob(pattern, &entry.remote) || glob(pattern, &entry.name))
        .flat_map(|(_, words)| words.iter().map(String::as_str))
        .collect()
}

/// PMA's tiers: whole-field exact, prefix and substring phrase matches, then
/// per-token exact, prefix/stem and substring matches, each times the field weight.
fn score(server: &Server, entry: &Entry, query: &str, query_tokens: &[String]) -> Option<usize> {
    let keywords = keywords(server, entry).join(" ");
    let fields = [
        (WEIGHT_NAME, entry.name.to_lowercase()),
        (WEIGHT_REMOTE, entry.remote.to_lowercase()),
        (WEIGHT_SERVER, server.name.to_lowercase()),
        (WEIGHT_DESCRIPTION, entry.description.to_lowercase()),
        (WEIGHT_KEYWORDS, keywords.to_lowercase()),
    ];
    let mut total = 0;
    let mut matched = HashSet::new();
    for (weight, value) in &fields {
        if value.is_empty() {
            continue;
        }
        if *value == query {
            total += weight * 14;
        } else if value.starts_with(query) {
            total += weight * 9;
        } else if value.contains(query) {
            total += weight * 6;
        }
        let field_tokens = tokens(value);
        for token in query_tokens {
            if field_tokens.contains(token) {
                total += weight * 4;
            } else if field_tokens.iter().any(|f| {
                f.starts_with(token.as_str())
                    || (f.len() >= MIN_STEM && token.starts_with(f.as_str()))
            }) {
                total += weight * 2;
            } else if value.contains(token.as_str()) {
                total += weight;
            } else {
                continue;
            }
            matched.insert(token);
        }
    }
    (!matched.is_empty()).then_some(total)
}

fn ranked<'a>(
    servers: &'a [Server],
    query: &str,
    only: Option<&str>,
) -> Vec<(usize, &'a Server, &'a Entry)> {
    let query = query.trim().to_lowercase();
    let query_tokens = tokens(&query);
    let mut hits: Vec<_> = servers
        .iter()
        .filter(|s| !matches!(s.status, ServerStatus::Disabled))
        .filter(|s| only.is_none_or(|name| s.name == name))
        .flat_map(|server| {
            let (query, query_tokens) = (&query, &query_tokens);
            server.entries.iter().filter_map(move |entry| {
                score(server, entry, query, query_tokens).map(|score| (score, server, entry))
            })
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.name.cmp(&b.2.name)));
    hits
}

fn search(
    servers: &[Server],
    query: &str,
    server: Option<&str>,
    limit: usize,
    offset: usize,
) -> AgentToolResult {
    let hits = ranked(servers, query, server);
    let page: Vec<_> = hits.iter().skip(offset).take(limit).collect();
    let mut text = if page.is_empty() {
        format!("No MCP tools match \"{}\".", clean(query))
    } else {
        format!(
            "{} of {} tools matching \"{}\":",
            page.len(),
            hits.len(),
            clean(query)
        )
    };
    for (_, server, entry) in &page {
        text.push('\n');
        text.push_str(&line(entry, server));
    }
    let next = offset + page.len();
    let next_offset = (next < hits.len()).then_some(next);
    if let Some(next) = next_offset {
        text.push_str(&format!(
            "\n\nMore: mcp({{ search: {}, offset: {next} }})",
            json!(query)
        ));
    }
    self::text(
        text,
        json!({
            "mode": "search",
            "total": hits.len(),
            "tools": page.iter().map(|(_, _, e)| &e.name).collect::<Vec<_>>(),
            "nextOffset": next_offset,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
async fn call(
    manager: &McpManager,
    servers: &[Server],
    name: &str,
    server: Option<&str>,
    params: &Value,
    call_id: &str,
    signal: CancellationToken,
    on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
) -> Result<AgentToolResult, AgentError> {
    if let Some(server) = server {
        if !servers.iter().any(|s| s.name == server) {
            return Ok(not_found_server(server));
        }
    }
    let (owner, entry) = match resolve(servers, name, server) {
        Found::One(owner, entry) => (owner, entry),
        found => return Ok(miss("call", name, servers, found)),
    };
    let args = match params.get("args") {
        None | Some(Value::Null) => json!({}),
        Some(Value::String(text)) => serde_json::from_str(text)
            .map_err(|e| AgentError::Tool(format!("`args` is not valid JSON: {e}")))?,
        Some(value) => value.clone(),
    };
    if !args.is_object() {
        return Err(AgentError::Tool("`args` must be a JSON object".into()));
    }
    let not_live = |reason: &str| {
        text(
            format!(
                "{} is {reason}. Use mcp({{ connect: \"{}\" }}) to retry.",
                clean(&owner.name),
                owner.name
            ),
            json!({"mode":"call","error":"not_connected","server":owner.name,"tool":entry.name}),
        )
    };
    match &owner.status {
        ServerStatus::Disabled => {
            return Ok(text(
                format!(
                    "Server \"{}\" is disabled. Enable it with /mcp enable {}.",
                    clean(&owner.name),
                    owner.name
                ),
                json!({"mode":"call","error":"server_disabled","server":owner.name}),
            ))
        }
        ServerStatus::Connecting => return Ok(not_live("still connecting")),
        ServerStatus::Failed(_) => return Ok(not_live("not connected")),
        ServerStatus::Connected { .. } => {}
    }
    let Some((tool, approval, ui)) = manager.live_tool(&owner.name, &entry.name) else {
        return Ok(not_live("no longer offering this tool"));
    };
    // A proxied call gets the same per-tool approval as a direct one.
    if approval {
        let Some(ui) = ui else {
            return Err(AgentError::Tool("MCP tool approval denied".into()));
        };
        approve(&ui, call_id, &entry.name, &signal)
            .await
            .map_err(AgentError::Tool)?;
    }
    tool.execute(call_id, args, signal, on_update).await
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
