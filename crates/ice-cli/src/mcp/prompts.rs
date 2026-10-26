use super::*;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Prompt {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub arguments: Vec<PromptArgument>,
}
impl Prompt {
    pub fn command_name(&self, server: &str) -> String {
        format!("/mcp__{server}__{}", prompt_name(&self.name))
    }
    fn resolve_args(&self, server: &str, input: &str) -> Result<BTreeMap<String, String>, String> {
        let (positional, named) = parse_args(input)?;
        let mut positional = positional.into_iter();
        let mut args = BTreeMap::new();
        for arg in &self.arguments {
            if let Some(value) = named.get(&arg.name).cloned().or_else(|| positional.next()) {
                if !value.is_empty() {
                    args.insert(arg.name.clone(), value);
                }
            }
        }
        args.extend(named);
        let missing: Vec<_> = self
            .arguments
            .iter()
            .filter(|arg| arg.required && args.get(&arg.name).is_none_or(String::is_empty))
            .map(|arg| arg.name.as_str())
            .collect();
        if !missing.is_empty() {
            let usage = self
                .arguments
                .iter()
                .map(|arg| {
                    if arg.required {
                        format!("<{}>", arg.name)
                    } else {
                        format!("[{}]", arg.name)
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            return Err(format!(
                "Missing required arguments: {}.\nUsage: {} {usage}",
                missing.join(", "),
                self.command_name(server)
            ));
        }
        Ok(args)
    }
}

fn prompt_name(name: &str) -> String {
    let mut cleaned = String::new();
    let mut replacing = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            cleaned.push(ch);
            replacing = false;
        } else if !replacing {
            cleaned.push('_');
            replacing = true;
        }
    }
    let cleaned = cleaned.trim_matches(['_', '-']);
    if cleaned.is_empty() {
        "prompt".into()
    } else if cleaned.starts_with(|ch: char| ch.is_ascii_digit()) {
        format!("_{cleaned}")
    } else {
        cleaned.into()
    }
}

fn parse_args(input: &str) -> Result<(Vec<String>, BTreeMap<String, String>), String> {
    let invalid = "Invalid prompt arguments: unmatched quote or escape";
    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut equals = None;
    // Keep token boundaries until shlex decodes them: quoted '=' is positional.
    for (index, ch) in input
        .char_indices()
        .chain(std::iter::once((input.len(), ' ')))
    {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(current) = quote {
            if ch == current {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == '=' && equals.is_none() {
            equals = Some(index);
        } else if ch.is_whitespace() {
            let raw = &input[start..index];
            if !raw.is_empty() {
                let token = shlex::split(raw)
                    .ok_or(invalid)?
                    .into_iter()
                    .next()
                    .ok_or(invalid)?;
                if let Some(eq) = equals.filter(|eq| *eq > start) {
                    let key = shlex::split(&input[start..eq]).ok_or(invalid)?.join("");
                    let value = shlex::split(&input[eq + 1..index]).ok_or(invalid)?.join("");
                    named.insert(key, value);
                } else {
                    positional.push(token);
                }
            }
            start = index + ch.len_utf8();
            equals = None;
        }
    }
    if quote.is_some() || escaped || start <= input.len() {
        return Err(invalid.into());
    }
    Ok((positional, named))
}

impl Client {
    pub(super) async fn list_prompts(&self) -> Result<Vec<Prompt>, String> {
        if !self.declares("prompts") {
            return Ok(Vec::new());
        }
        let mut prompts = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        for _ in 0..100 {
            let params = cursor
                .as_ref()
                .map(|c| json!({"cursor":c}))
                .unwrap_or_else(|| json!({}));
            let page = self
                .request(
                    "prompts/list",
                    params,
                    CancellationToken::new(),
                    START_TIMEOUT,
                )
                .await?;
            let items = page
                .get("prompts")
                .and_then(Value::as_array)
                .ok_or("MCP prompts/list missing prompts")?;
            for item in items {
                let mut prompt: Prompt = serde_json::from_value(item.clone())
                    .map_err(|error| format!("Invalid MCP prompt: {error}"))?;
                prompt.description.retain(|c| !c.is_control());
                if !names.insert(prompt_name(&prompt.name)) {
                    tracing::warn!(name = %prompt.name, "Skipping colliding MCP prompt command");
                    continue;
                }
                prompts.push(prompt);
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match &cursor {
                Some(cursor) if !cursors.insert(cursor.clone()) => {
                    return Err("MCP repeated prompt pagination cursor".into())
                }
                None => return Ok(prompts),
                _ => {}
            }
        }
        Err("MCP prompts/list exceeded 100 pages".into())
    }
}

pub(super) async fn get_prompt(
    client: &Client,
    server: &str,
    prompt: &Prompt,
    input: &str,
) -> Result<String, String> {
    let args = prompt.resolve_args(server, input)?;
    let result = client
        .request(
            "prompts/get",
            json!({"name":prompt.name,"arguments":args}),
            CancellationToken::new(),
            CALL_TIMEOUT,
        )
        .await?;
    format_result(result)
}

fn format_result(result: Value) -> Result<String, String> {
    let messages = result
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("MCP prompts/get missing messages")?;
    let mut lines = Vec::new();
    for message in messages {
        let content = &message["content"];
        let string = |key: &str| content[key].as_str().unwrap_or("");
        let text = match string("type") {
            "text" => string("text").to_owned(),
            "resource" => {
                let resource = &content["resource"];
                let uri = resource["uri"].as_str().unwrap_or("");
                match resource["text"].as_str() {
                    Some(text) => format!("[resource {uri}]\n{text}"),
                    None => format!("[resource {uri}]"),
                }
            }
            "resource_link" => format!(
                "[resource_link {}{}]",
                string("uri"),
                content["name"]
                    .as_str()
                    .map(|name| format!(" — {name}"))
                    .unwrap_or_default()
            ),
            "image" => format!(
                "[image {}{}]",
                content["mimeType"].as_str().unwrap_or("unknown"),
                if string("data").is_empty() {
                    ""
                } else {
                    " (embedded)"
                }
            ),
            "audio" => format!(
                "[audio {}]",
                content["mimeType"].as_str().unwrap_or("unknown")
            ),
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }
        let role = message["role"]
            .as_str()
            .ok_or("MCP prompt message missing role")?;
        if role != "user" && role != "assistant" {
            return Err("MCP prompt message has invalid role".into());
        }
        lines.push(if role == "user" && messages.len() == 1 {
            text
        } else {
            format!("[{role}] {text}")
        });
    }
    let text = lines.join("\n\n").trim().to_owned();
    if text.is_empty() {
        return Err("MCP prompt returned no text".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prompt() -> Prompt {
        serde_json::from_value(
            json!({"name":"Brief","arguments":[{"name":"day","required":true},{"name":"topic"}]}),
        )
        .unwrap()
    }
    #[test]
    fn names_match_pma() {
        for (name, expected) in [
            ("Review-Code", "Review-Code"),
            ("__A  B!!", "A_B"),
            ("9", "_9"),
            ("💧", "prompt"),
            ("", "prompt"),
        ] {
            assert_eq!(prompt_name(name), expected);
        }
    }
    #[test]
    fn positional_named_mixed_and_quoted_arguments() {
        for (input, expected) in [
            (
                "today 'important  tasks'",
                json!({"day":"today","topic":"important  tasks"}),
            ),
            (
                "day=today topic=\"important tasks\"",
                json!({"day":"today","topic":"important tasks"}),
            ),
            (
                "topic=work today extra=ok",
                json!({"day":"today","topic":"work","extra":"ok"}),
            ),
            ("day=today work", json!({"day":"today","topic":"work"})),
            ("'a=b'", json!({"day":"a=b"})),
            (r"a\=b", json!({"day":"a=b"})),
            ("today 'a=b'", json!({"day":"today","topic":"a=b"})),
            ("day=first day=last", json!({"day":"last"})),
        ] {
            assert_eq!(
                json!(prompt().resolve_args("demo", input).unwrap()),
                expected
            );
        }
        for input in ["", "day=", "''"] {
            let error = prompt().resolve_args("demo", input).unwrap_err();
            assert!(error.contains("Usage: /mcp__demo__Brief <day> [topic]"));
        }
        assert!(prompt().resolve_args("demo", "'unfinished").is_err());
    }
    #[test]
    fn flatten_preserves_roles_and_omits_binary_payloads() {
        assert_eq!(
            format_result(
                json!({"messages":[{"role":"user","content":{"type":"text","text":"hi"}}]})
            )
            .unwrap(),
            "hi"
        );
        assert_eq!(format_result(json!({"messages":[
            {"role":"user","content":{"type":"text","text":"hi"}},
            {"role":"assistant","content":{"type":"resource","resource":{"uri":"test:///a","text":"answer"}}},
            {"role":"user","content":{"type":"image","mimeType":"image/png","data":"secret"}}
        ]})).unwrap(), "[user] hi\n\n[assistant] [resource test:///a]\nanswer\n\n[user] [image image/png (embedded)]");
        assert!(format_result(json!({"messages":[]})).is_err());
        assert!(format_result(json!({})).is_err());
    }
}
