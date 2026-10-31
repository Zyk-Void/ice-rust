//! Minimal P0 JSONL protocol for the native icers RPC host.
use serde_json::{json, Value};

/// Allow bounded base64 image attachments as well as text/editor context.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Decode and validate inline images before a run is accepted. Never resolve
/// image URLs here: only bytes explicitly supplied by the client are trusted.
pub(crate) fn prompt_images(
    value: Option<&Value>,
) -> Result<Vec<ice_ai::types::ImageContent>, String> {
    use base64::Engine as _;
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let images: Vec<ice_ai::types::ImageContent> = serde_json::from_value(value.clone())
        .map_err(|_| "images must be an array of base64 images with mimeType".to_string())?;
    for image in &images {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data)
            .map_err(|_| "image data must be valid base64".to_string())?;
        let detected = ice_tools::detect_supported_image_mime_type(&bytes)
            .ok_or("unsupported or empty image data")?;
        if detected != image.mime_type {
            return Err(format!("image mimeType must match its data ({detected})"));
        }
    }
    Ok(images)
}
pub const PI_BASELINE: &str = "1.2.0";

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(flatten)]
    pub command: Command,
}

/// Payload schema for `Request`. Only deserialization matters: the dispatcher
/// matches on the raw `type` string, so the parsed fields are never read.
#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Prompt {
        #[serde(alias = "content")]
        message: String,
    },
    Abort,
    GetState,
    GetCommands,
    /// Read-only snapshot of the current branch's message entries.
    GetMessages {
        limit: Option<u64>,
        #[serde(rename = "beforeId")]
        before_id: Option<String>,
    },
    GetHistory {
        limit: Option<u64>,
        #[serde(rename = "afterSeq")]
        after_seq: Option<u64>,
        #[serde(rename = "beforeSeq")]
        before_seq: Option<u64>,
    },
    GetStats,
    GetQueue,
    QueueMessage {
        message: String,
        #[serde(rename = "deliverAs")]
        deliver_as: Option<String>,
    },
    CancelQueued {
        #[serde(rename = "entryId")]
        entry_id: String,
    },
    ClearQueue,
    Navigate {
        #[serde(rename = "targetId")]
        target_id: Option<String>,
    },
    /// Continue the run that recovery left interrupted.
    ResumePending,
    /// Rewrite the model's view of the branch into a summary.
    Compact {
        #[serde(rename = "customInstructions")]
        custom_instructions: Option<String>,
    },
    GetAvailableModels,
    GetAuthStatus,
    Oauth {
        provider: String,
        action: String,
        input: Option<Value>,
    },
    SetApiKey {
        provider: String,
        #[serde(rename = "apiKey")]
        api_key: String,
    },
    SetModel {
        provider: Option<String>,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    GetAvailableThinkingLevels,
    SetThinkingLevel {
        level: ice_ai::types::ThinkingLevel,
    },
    GetAvailableTools,
    SetActiveTools {
        tools: Vec<String>,
    },
    UiDialogResponse {
        #[serde(rename = "requestId")]
        request_id: String,
        answer: Option<Value>,
        #[serde(default)]
        cancelled: bool,
    },
    Ping,
    Stop,
}

pub const SUPPORTED_COMMANDS: &[&str] = &[
    "get_state",
    "get_commands",
    "get_messages",
    "get_history",
    "get_stats",
    "get_queue",
    "queue_message",
    "cancel_queued",
    "clear_queue",
    "navigate",
    "prompt",
    "resume_pending",
    "compact",
    "abort",
    "get_available_models",
    "get_auth_status",
    "oauth",
    "set_api_key",
    "set_model",
    "get_available_thinking_levels",
    "set_thinking_level",
    "get_available_tools",
    "set_active_tools",
    "ui_dialog_response",
    "ping",
    "stop",
];

pub fn response(id: Option<Value>, command: &str, result: Result<Option<Value>, String>) -> Value {
    let mut out = json!({
        "type": "response",
        "command": command,
        "success": result.is_ok(),
        "status": if result.is_ok() { "ok" } else { "error" },
    });
    if let Some(id) = id {
        out["id"] = id;
    }
    match result {
        Ok(Some(data)) => {
            out["result"] = data.clone();
            out["data"] = data;
        }
        Ok(None) => {}
        Err(error) => out["error"] = json!(error),
    }
    out
}

pub fn ready() -> Value {
    json!({
        "type": "ready",
        "protocol": "icers",
        "piCompatibilityTarget": PI_BASELINE,
        "supportedCommands": SUPPORTED_COMMANDS,
        "semantics": {
            "promptResponse": "accepted",
            "settlementEvent": "agent_settled",
            "agentEndIsTerminal": false
        },
        "limits": {"frameBytes": MAX_FRAME_BYTES, "maxConcurrentPrompts": 1},
        "parityComplete": false
    })
}

/// Only expose capability metadata, never provider credentials or headers.
pub fn model_descriptor(model: &ice_ai::Model) -> Value {
    json!({
        "id": model.id,
        "name": model.name,
        "api": model.api,
        "provider": model.provider,
        "reasoning": model.reasoning,
        "input": model.input,
        "contextWindow": model.context_window,
        "maxTokens": model.max_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_advertises_the_p0_contract() {
        let value = ready();
        assert_eq!(value["protocol"], "icers");
        assert_eq!(value["semantics"]["settlementEvent"], "agent_settled");
        assert!(value["supportedCommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "prompt"));
    }

    #[test]
    fn ready_advertises_the_read_only_history_snapshot() {
        let value = ready();
        assert!(value["supportedCommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "get_messages"));
    }

    #[test]
    fn ready_advertises_resuming_an_interrupted_run() {
        let value = ready();
        assert!(value["supportedCommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "resume_pending"));
    }

    #[test]
    fn ready_advertises_compaction() {
        let value = ready();
        assert!(value["supportedCommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "compact"));
    }

    #[test]
    fn compact_payload_carries_instructions() {
        let request: Request = serde_json::from_value(json!({
            "type": "compact", "id": 5, "customInstructions": "keep the decisions"
        }))
        .unwrap();
        assert!(matches!(
            request.command,
            Command::Compact {
                custom_instructions: Some(_)
            }
        ));
    }

    #[test]
    fn resume_pending_is_a_bare_command() {
        let request: Request =
            serde_json::from_value(json!({"type": "resume_pending", "id": 4})).unwrap();
        assert!(matches!(request.command, Command::ResumePending));
    }

    #[test]
    fn get_messages_payload_keeps_its_limit_and_cursor() {
        let request: Request = serde_json::from_value(json!({
            "type": "get_messages", "id": 3, "limit": 25, "beforeId": "entry-12"
        }))
        .unwrap();
        assert!(matches!(
            request.command,
            Command::GetMessages {
                limit: Some(25),
                before_id: Some(_)
            }
        ));
    }

    #[test]
    fn oauth_payload_accepts_provider_actions_and_optional_input() {
        for action in ["begin", "exchange", "refresh", "revoke"] {
            let request: Request = serde_json::from_value(json!({
                "type": "oauth", "provider": "fixture", "action": action,
                "input": {"code": "fixture-code"}
            }))
            .unwrap();
            assert!(matches!(request.command, Command::Oauth {
                provider, action: parsed_action, input: Some(input)
            } if provider == "fixture" && parsed_action == action
                && input == json!({"code": "fixture-code"})));
        }
        let request: Request = serde_json::from_value(json!({
            "type": "oauth", "provider": "fixture", "action": "begin"
        }))
        .unwrap();
        assert!(matches!(
            request.command,
            Command::Oauth { input: None, .. }
        ));
    }

    #[test]
    fn oauth_payload_rejects_missing_or_wrongly_typed_required_fields() {
        for payload in [
            json!({"type": "oauth", "action": "begin"}),
            json!({"type": "oauth", "provider": "fixture"}),
            json!({"type": "oauth", "provider": 1, "action": "begin"}),
            json!({"type": "oauth", "provider": "fixture", "action": false}),
            json!({"type": "oauth", "provider": null, "action": "begin"}),
            json!({"type": "oauth", "provider": "fixture", "action": null}),
        ] {
            assert!(
                serde_json::from_value::<Request>(payload.clone()).is_err(),
                "{payload}"
            );
        }
    }

    #[test]
    fn auth_commands_keep_secret_fields_in_request_only() {
        let request: Request = serde_json::from_value(json!({
            "type": "set_api_key",
            "id": 8,
            "provider": "anthropic",
            "apiKey": "secret-fixture"
        }))
        .unwrap();
        assert!(matches!(
            request.command,
            Command::SetApiKey { provider, api_key }
                if provider == "anthropic" && api_key == "secret-fixture"
        ));
        let status = response(
            Some(json!(8)),
            "set_api_key",
            Ok(Some(json!({
                "provider": "anthropic",
                "ready": true,
                "source": "auth.json",
                "requiresRestart": true
            }))),
        );
        assert!(!status.to_string().contains("secret-fixture"));
    }
    #[test]
    fn response_has_canonical_success_data_shape() {
        let value = response(
            Some(json!(7)),
            "prompt",
            Ok(Some(json!({"accepted": true}))),
        );
        assert_eq!(value["type"], "response");
        assert_eq!(value["success"], true);
        assert_eq!(value["data"]["accepted"], true);
    }
}
