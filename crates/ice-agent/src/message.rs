//! Mirrors `packages/agent/src/types.ts` (open enum portion) — the
//! `AgentMessage = Message | Custom` declaration-merging model.
//!
//! TS lets apps add custom message kinds via declaration merging on
//! `CustomAgentMessages`. Rust has no declaration merging, so we model the same
//! openness as a four-arm enum: the three base [`Message`] arms plus
//! [`CustomMessage`]. Apps extend by populating `Custom`, not by adding arms.
//!
//! Serialization mirrors the wire shape: `#[serde(tag="kind")]` with
//! `{kind:"user"|"assistant"|"toolResult"|"custom", ...}`. The three base arms
//! flatten their `Message` (which itself is tagged on `role`); we serialize
//! them via a custom impl so the outer `kind` discriminator stays consistent
//! and the inner role-specific fields survive a round-trip. `Custom` carries a
//! free-form `data: Value` so each role's structured payload rides along.

use ice_ai::types::{
    AssistantMessage, Content, Message, PromptOrigin, ToolResultMessage, UserMessage,
};
use serde::{Deserialize, Serialize};

/// `AgentMessage = Message | Custom`. The open-enum port of TS
/// `AgentMessage = Message | CustomAgentMessages[keyof CustomAgentMessages]`.
///
/// `PartialEq` is derived so `pi-harness` entry equality (and tests) can compare
/// persisted messages by value. `ice_ai`'s base message types already implement
/// `PartialEq`; `CustomMessage` does too, so the four-arm enum derives cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentMessage {
    #[serde(with = "user_with_origin")]
    User(UserMessage),
    Assistant(Box<AssistantMessage>),
    ToolResult(Box<ToolResultMessage>),
    Custom(CustomMessage),
}

mod user_with_origin {
    use super::*;
    use serde::ser::SerializeMap;

    pub fn serialize<S: serde::Serializer>(
        user: &UserMessage,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("role", &user.role)?;
        map.serialize_entry("content", &user.content)?;
        map.serialize_entry("timestamp", &user.timestamp)?;
        if let Some(origin) = user
            .origin
            .as_ref()
            .filter(|_| user.original_prompt().is_some())
        {
            map.serialize_entry("icersPromptOrigin", origin)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<UserMessage, D::Error> {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let origin = value
            .as_object_mut()
            .and_then(|object| object.remove("icersPromptOrigin"));
        let mut user: UserMessage =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        if let Some(origin) = origin {
            let origin: PromptOrigin =
                serde_json::from_value(origin).map_err(serde::de::Error::custom)?;
            user.origin = Some(origin);
            if user.original_prompt().is_none() {
                user.origin = None;
            }
        }
        Ok(user)
    }
}

/// Borrow a validated raw prompt from a durable agent message.
pub fn original_user_prompt(message: &AgentMessage) -> Option<&str> {
    match message {
        AgentMessage::User(user) => user.original_prompt(),
        _ => None,
    }
}

impl AgentMessage {
    /// The logical role tag — mirrors `message.role` on the TS side. For base
    /// messages this is `"user"`/`"assistant"`/`"toolResult"`; for custom it is
    /// the custom role string. Used by the loop's role checks (e.g. "can't
    /// continue from an assistant message").
    pub fn role(&self) -> AgentMessageRole {
        match self {
            AgentMessage::User(_) => AgentMessageRole::User,
            AgentMessage::Assistant(_) => AgentMessageRole::Assistant,
            AgentMessage::ToolResult(_) => AgentMessageRole::ToolResult,
            AgentMessage::Custom(c) => AgentMessageRole::Custom(c.role.clone()),
        }
    }

    pub fn is_assistant(&self) -> bool {
        matches!(self, AgentMessage::Assistant(_))
    }

    /// If this is an assistant message, borrow it.
    pub fn as_assistant(&self) -> Option<&AssistantMessage> {
        match self {
            AgentMessage::Assistant(a) => Some(a),
            _ => None,
        }
    }

    /// If this is an assistant message, own a clone of it.
    pub fn into_assistant(self) -> Option<AssistantMessage> {
        match self {
            AgentMessage::Assistant(a) => Some(*a),
            _ => None,
        }
    }
}

/// Serialize an `AssistantMessage` in the exact shape `AgentMessage::Assistant`
/// produces (`{"kind":"assistant", …}`), without materializing an owned
/// `AgentMessage` copy of it.
///
/// [`crate::AgentEvent::MessageUpdate`] carries the streaming partial as an
/// `Arc<AssistantMessage>` so neither the loop nor its consumers deep-clone the
/// whole growing message per delta. Consumers that need the message as JSON (the
/// remote protocol, the extension bridge) use this to keep the wire/plugin shape
/// byte-identical to a real `AgentMessage` — a deep clone here would reintroduce
/// exactly the per-delta copy the `Arc` exists to avoid. The equivalence is
/// pinned by `assistant_json_matches_the_agent_message_shape`.
///
/// [`crate::AgentEvent::MessageUpdate`]: crate::events::AgentEvent::MessageUpdate
pub fn assistant_json(message: &AssistantMessage) -> serde_json::Value {
    /// Internally tagged exactly like `AgentMessage`, so serde emits the same
    /// `"kind"` discriminator first and then the message fields.
    #[derive(serde::Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum Tagged<'a> {
        Assistant(&'a AssistantMessage),
    }
    serde_json::to_value(Tagged::Assistant(message)).unwrap_or(serde_json::Value::Null)
}

impl From<UserMessage> for AgentMessage {
    fn from(m: UserMessage) -> Self {
        AgentMessage::User(m)
    }
}
impl From<AssistantMessage> for AgentMessage {
    fn from(m: AssistantMessage) -> Self {
        AgentMessage::Assistant(Box::new(m))
    }
}
impl From<ToolResultMessage> for AgentMessage {
    fn from(m: ToolResultMessage) -> Self {
        AgentMessage::ToolResult(Box::new(m))
    }
}
impl From<Message> for AgentMessage {
    fn from(m: Message) -> Self {
        match m {
            Message::User(u) => AgentMessage::User(u),
            Message::Assistant(a) => AgentMessage::Assistant(a),
            Message::ToolResult(t) => AgentMessage::ToolResult(t),
        }
    }
}

/// Typed view of `AgentMessage::role`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMessageRole {
    User,
    Assistant,
    ToolResult,
    Custom(String),
}

impl AgentMessageRole {
    pub fn as_str(&self) -> &str {
        match self {
            AgentMessageRole::User => "user",
            AgentMessageRole::Assistant => "assistant",
            AgentMessageRole::ToolResult => "toolResult",
            AgentMessageRole::Custom(s) => s.as_str(),
        }
    }
}

/// A custom app message. `role` is the app-defined tag (e.g. `"bashExecution"`,
/// `"branchSummary"`); `content` is provider-content the renderer emits;
/// `data` carries the role's structured payload; `timestamp` is ms-since-epoch.
///
/// `pi-harness::messages` provides typed constructors per known role;
/// `pi-agent` stays role-agnostic and only owns this shell.
///
/// `PartialEq` is derived so `pi-harness` entry equality (and tests) can compare
/// persisted custom messages by value. `serde_json::Value` + `Content` both
/// implement `PartialEq`, so the derive is sound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessage {
    pub role: String,
    pub content: Vec<Content>,
    pub data: serde_json::Value,
    pub timestamp: i64,
}

impl CustomMessage {
    pub fn new(
        role: impl Into<String>,
        content: Vec<Content>,
        data: serde_json::Value,
        timestamp: i64,
    ) -> Self {
        Self {
            role: role.into(),
            content,
            data,
            timestamp,
        }
    }
}

#[cfg(test)]
mod prompt_origin_tests {
    use super::*;

    #[test]
    fn serde_agent_message_preserves_validated_origin() {
        let message = AgentMessage::User(
            UserMessage::new("expanded", 12).with_origin("$review", "skill-expansion-v1"),
        );
        let encoded = serde_json::to_value(&message).unwrap();
        assert_eq!(encoded["kind"], "user");
        assert_eq!(encoded["icersPromptOrigin"]["rawText"], "$review");
        let decoded: AgentMessage = serde_json::from_value(encoded).unwrap();
        assert_eq!(original_user_prompt(&decoded), Some("$review"));
    }

    #[test]
    fn tampered_agent_message_drops_origin_and_legacy_loads() {
        let message =
            AgentMessage::User(UserMessage::new("expanded", 12).with_origin("raw", "test"));
        let mut encoded = serde_json::to_value(message).unwrap();
        encoded["content"] = serde_json::json!("tampered");
        let decoded: AgentMessage = serde_json::from_value(encoded).unwrap();
        assert_eq!(original_user_prompt(&decoded), None);
        let legacy = serde_json::json!({"kind":"user","role":"user","content":"old","timestamp":1});
        let decoded: AgentMessage = serde_json::from_value(legacy).unwrap();
        assert_eq!(original_user_prompt(&decoded), None);
    }
}

#[cfg(test)]
mod assistant_json_tests {
    use super::*;
    use ice_ai::types::{Api, AssistantMessage};

    #[test]
    fn assistant_json_matches_the_agent_message_shape() {
        // The whole point of `assistant_json` is that consumers can keep
        // serializing a `MessageUpdate` partial the way they always have, so it
        // must be indistinguishable from the real `AgentMessage` encoding.
        let mut assistant =
            AssistantMessage::empty(Api::OpenaiCompletions, "gateway", "gpt-test", 7);
        assistant.error_message = Some("boom".into());
        let as_agent = AgentMessage::Assistant(Box::new(assistant.clone()));
        assert_eq!(
            assistant_json(&assistant),
            serde_json::to_value(&as_agent).expect("AgentMessage serializes"),
        );
    }
}
