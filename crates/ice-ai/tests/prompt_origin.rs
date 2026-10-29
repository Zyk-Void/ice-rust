use ice_ai::types::{Message, UserMessage};

#[test]
fn provider_message_serializers_never_include_raw_origin() {
    let user =
        UserMessage::new("expanded instruction", 42).with_origin("$review", "skill-expansion-v1");
    let direct = serde_json::to_value(&user).unwrap();
    let message = serde_json::to_value(Message::User(user)).unwrap();
    for value in [direct, message] {
        assert!(!value.to_string().contains("$review"));
        assert!(value.get("icersPromptOrigin").is_none());
    }
}

#[test]
fn direct_user_deserialization_does_not_trust_origin() {
    let user = UserMessage::new("expanded", 1).with_origin("raw", "test");
    let mut value = serde_json::to_value(user).unwrap();
    value["icersPromptOrigin"] = serde_json::json!({"rawText":"forged"});
    let loaded: UserMessage = serde_json::from_value(value).unwrap();
    assert_eq!(loaded.origin, None);
}

#[test]
fn origin_validation_detects_changed_text_and_timestamp() {
    let user = UserMessage::new("expanded", 1).with_origin("raw", "skill-expansion-v1");
    assert_eq!(user.original_prompt(), Some("raw"));
    let mut changed_text = user.clone();
    changed_text.content = "changed".into();
    assert_eq!(changed_text.original_prompt(), None);
    let mut changed_raw = user.clone();
    changed_raw.origin.as_mut().unwrap().raw_text = "forged".into();
    assert_eq!(changed_raw.original_prompt(), None);
    let mut changed_timestamp = user;
    changed_timestamp.timestamp += 1;
    assert_eq!(changed_timestamp.original_prompt(), None);
}
