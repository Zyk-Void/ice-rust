//! `sampling/createMessage`: a server asks the user's model for one completion.
//! Text only, no tools, and the user confirms both the request and the reply
//! unless `samplingAutoApprove` is set, since the call spends the user's quota.
use super::elicitation::{clean, invalid};
use super::*;
use ice_ai::types::{
    AssistantMessage, Content, Context, Message, StopReason, TextContent, UserMessage,
};
use ice_ai::{Model, Provider, SimpleStreamOptions};
use std::sync::RwLock;

const PREVIEW_CHARS: usize = 1000;
/// MCP's code for a request the user refused.
const USER_REJECTED: i64 = -1;

pub(super) type Providers = Arc<RwLock<Vec<Arc<dyn Provider>>>>;
/// The session's active model, read per request so `/model` switches apply.
pub(super) type CurrentModel = Arc<dyn Fn() -> BoxFuture<'static, Option<Model>> + Send + Sync>;

pub(super) fn register(
    handlers: Handlers,
    server: &str,
    ui: UiDialogMailbox,
    providers: Providers,
    current_model: CurrentModel,
    auto_approve: bool,
) -> Handlers {
    let state = Arc::new(Sampling {
        server: server.into(),
        ui,
        providers,
        current_model,
        auto_approve,
    });
    handlers.request(
        "sampling/createMessage",
        json!({}),
        Arc::new(move |params, signal| {
            let state = state.clone();
            Box::pin(async move { state.handle(params, &signal).await })
        }),
    )
}

struct Sampling {
    server: String,
    ui: UiDialogMailbox,
    providers: Providers,
    current_model: CurrentModel,
    auto_approve: bool,
}

struct Request {
    system: Option<String>,
    /// `(is_assistant, text)` in conversation order.
    messages: Vec<(bool, String)>,
    max_tokens: u64,
    hints: Vec<String>,
    temperature: Option<f64>,
}

fn rejected(message: impl Into<String>) -> RpcError {
    RpcError {
        code: USER_REJECTED,
        message: message.into(),
    }
}
fn failed(message: impl Into<String>) -> RpcError {
    RpcError {
        code: -32603,
        message: message.into(),
    }
}
fn preview(text: &str) -> String {
    let text = clean(text);
    match text.char_indices().nth(PREVIEW_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

fn text_of(content: &Value) -> Result<String, RpcError> {
    let blocks = match content {
        Value::Array(blocks) => blocks.as_slice(),
        single => std::slice::from_ref(single),
    };
    let mut parts = Vec::new();
    for block in blocks {
        match (block["type"].as_str(), block["text"].as_str()) {
            (Some("text"), Some(text)) => parts.push(text),
            _ => return Err(invalid("Only text sampling content is supported")),
        }
    }
    Ok(parts.join("\n"))
}

fn parse(params: &Value) -> Result<Request, RpcError> {
    if params
        .get("includeContext")
        .is_some_and(|v| !v.is_null() && v != "none")
    {
        return Err(invalid("includeContext is not supported"));
    }
    if ["tools", "toolChoice"]
        .iter()
        .any(|key| params.get(key).is_some_and(|v| !v.is_null()))
    {
        return Err(invalid("Tools in sampling requests are not supported"));
    }
    if params["stopSequences"]
        .as_array()
        .is_some_and(|sequences| !sequences.is_empty())
    {
        return Err(invalid("stopSequences are not supported"));
    }
    let max_tokens = params["maxTokens"]
        .as_u64()
        .filter(|tokens| *tokens > 0)
        .ok_or_else(|| invalid("maxTokens must be a positive integer"))?;
    let messages = params["messages"]
        .as_array()
        .filter(|messages| !messages.is_empty())
        .ok_or_else(|| invalid("Missing sampling messages"))?
        .iter()
        .map(|message| {
            let assistant = match message["role"].as_str() {
                Some("user") => false,
                Some("assistant") => true,
                _ => return Err(invalid("Sampling message role must be user or assistant")),
            };
            Ok((assistant, text_of(&message["content"])?))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let hints = params["modelPreferences"]["hints"]
        .as_array()
        .map(|hints| {
            hints
                .iter()
                .filter_map(|hint| hint["name"].as_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    Ok(Request {
        system: params["systemPrompt"].as_str().map(str::to_owned),
        messages,
        max_tokens,
        hints,
        temperature: params["temperature"].as_f64(),
    })
}

/// First hint that is a substring of a catalog model id wins; otherwise the
/// session's current model. The provider must be one the session can call.
fn resolve(
    providers: &[Arc<dyn Provider>],
    hints: &[String],
    current: Option<Model>,
) -> Option<(Arc<dyn Provider>, Model)> {
    let hinted = hints.iter().find_map(|hint| {
        providers.iter().find_map(|provider| {
            provider
                .models()
                .iter()
                .find(|model| model.id.to_lowercase().contains(hint.as_str()))
                .cloned()
        })
    });
    let model = hinted.or(current)?;
    let provider = providers
        .iter()
        .find(|provider| provider.id() == model.provider)?
        .clone();
    Some((provider, model))
}

fn context(request: &Request, model: &Model) -> Context {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64);
    let text = |text: &str| {
        Content::Text(TextContent {
            kind: Default::default(),
            text: text.into(),
            text_signature: None,
        })
    };
    let messages = request
        .messages
        .iter()
        .map(|(assistant, body)| {
            if *assistant {
                let mut message = AssistantMessage::empty(
                    model.api.clone(),
                    model.provider.clone(),
                    model.id.clone(),
                    now,
                );
                message.content = vec![text(body)];
                message.stop_reason = StopReason::Stop;
                Message::Assistant(Box::new(message))
            } else {
                Message::User(UserMessage::new(body.as_str(), now))
            }
        })
        .collect();
    Context {
        system_prompt: request.system.clone(),
        messages,
        tools: Vec::new(),
    }
}

impl Sampling {
    async fn allow(
        &self,
        summary: String,
        denied: &str,
        signal: &CancellationToken,
    ) -> Result<(), RpcError> {
        if self.auto_approve {
            return Ok(());
        }
        let call = format!("mcp-sampling-{}", uuid::Uuid::new_v4());
        confirm(&self.ui, &call, &summary, denied, signal)
            .await
            .map_err(rejected)
    }
    async fn handle(&self, params: Value, signal: &CancellationToken) -> Result<Value, RpcError> {
        let request = parse(&params)?;
        let current = (self.current_model)().await;
        let providers = self
            .providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let (provider, model) = resolve(&providers, &request.hints, current)
            .ok_or_else(|| failed("No model available for sampling"))?;
        let asked = request
            .messages
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        self.allow(
            format!(
                "MCP server {} wants a completion from {}/{} (up to {} tokens):\n\n{}",
                clean(&self.server),
                clean(&model.provider),
                clean(&model.id),
                request.max_tokens,
                preview(&asked)
            ),
            "Sampling request denied by user",
            signal,
        )
        .await?;
        let options = SimpleStreamOptions {
            signal: signal.clone(),
            max_tokens: Some(request.max_tokens),
            temperature: request.temperature,
            ..Default::default()
        };
        let reply = provider
            .stream_simple(&model, &context(&request, &model), &options)
            .await
            .result()
            .await
            .map_err(|_| failed("Sampling stream ended without a result"))?;
        let stop_reason = match reply.stop_reason {
            StopReason::Stop => "endTurn",
            StopReason::Length => "maxTokens",
            StopReason::Aborted => return Err(failed("Sampling request cancelled")),
            _ => {
                return Err(failed(
                    reply
                        .error_message
                        .unwrap_or_else(|| "Sampling request failed".into()),
                ))
            }
        };
        let text = crate::agent_session::assistant_text(&reply);
        if text.is_empty() {
            return Err(failed("Model returned no text"));
        }
        self.allow(
            format!(
                "Return this response to MCP server {}?\n\n{}",
                clean(&self.server),
                preview(&text)
            ),
            "Sampling response rejected by user",
            signal,
        )
        .await?;
        Ok(json!({
            "role": "assistant",
            "content": {"type": "text", "text": text},
            "model": reply.model,
            "stopReason": stop_reason,
        }))
    }
}

#[cfg(test)]
mod tests;
