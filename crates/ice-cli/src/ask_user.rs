//! Shared parsing for native ask-user dialogs.

/// Parsed presentation data for one `ask_user` question. Supports both the
/// flat native question contract and the earlier `questions[0]` alias, plus a
/// `confirm` summary.
#[derive(Clone)]
pub(crate) struct AskUserPrompt {
    pub(crate) question_id: String,
    pub(crate) question: String,
    pub(crate) header: Option<String>,
    pub(crate) context: Option<String>,
    pub(crate) options: Vec<(String, Option<String>)>,
    pub(crate) allow_multiple: bool,
    pub(crate) allow_freeform: bool,
    pub(crate) suggest: Option<String>,
    pub(crate) kind: String,
}

fn ask_user_str(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn ask_user_bool(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_bool))
}

fn ask_user_options(value: &serde_json::Value) -> Vec<(String, Option<String>)> {
    let mut options = Vec::new();
    let Some(items) = value.get("options").and_then(serde_json::Value::as_array) else {
        return options;
    };
    for item in items {
        if let Some(title) = item.as_str() {
            let title = title.trim();
            if !title.is_empty() {
                options.push((title.to_string(), None));
            }
            continue;
        }
        let Some(object) = item.as_object() else {
            continue;
        };
        let title = ["title", "label", "text", "value", "name"]
            .iter()
            .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|title| !title.is_empty());
        let Some(title) = title else { continue };
        let description = object
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string);
        options.push((title.to_string(), description));
    }
    options
}

pub(crate) fn parse_ask_user_prompt(request: &ice_extensions::UiDialogRequest) -> AskUserPrompt {
    let ui = &request.ui;
    let kind = ui
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("selector")
        .to_string();
    // A request may carry a single flat question or a `questions[]` array. The
    // plugin emits one question per request, but accepting `questions[0]` keeps
    // the host compatible with hosts/plugins that batch.
    let nested = ui
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first());
    let source = nested.filter(|value| value.is_object()).unwrap_or(ui);

    let question = ask_user_str(source, &["question", "summary", "title"])
        .or_else(|| ask_user_str(ui, &["question", "summary", "title"]))
        .unwrap_or_default();
    let question_id = ask_user_str(source, &["id", "questionId"])
        .or_else(|| ask_user_str(ui, &["id", "questionId"]))
        .unwrap_or_else(|| request.request_id.clone());
    let header = ask_user_str(source, &["header"]).or_else(|| ask_user_str(ui, &["header"]));
    let context = ask_user_str(source, &["context", "message"])
        .or_else(|| ask_user_str(ui, &["context", "message"]));
    let mut options = ask_user_options(source);
    if options.is_empty() {
        options = ask_user_options(ui);
    }
    if kind == "confirm" && options.is_empty() {
        options = vec![("Yes".to_string(), None), ("No".to_string(), None)];
    }
    let allow_multiple = ask_user_bool(source, &["allowMultiple", "allow_multiple", "multiple"])
        .or_else(|| ask_user_bool(ui, &["allowMultiple", "allow_multiple", "multiple"]))
        .unwrap_or(false);
    // Freeform defaults on when there is nothing to pick from, matching the
    // "no options ⇒ free input" contract.
    let allow_freeform =
        ask_user_bool(source, &["allowFreeform", "allow_freeform", "allow_custom"])
            .or_else(|| ask_user_bool(ui, &["allowFreeform", "allow_freeform", "allow_custom"]))
            .unwrap_or(options.is_empty());
    let suggest = ask_user_str(source, &["suggest", "placeholder"])
        .or_else(|| ask_user_str(ui, &["suggest", "placeholder"]));

    AskUserPrompt {
        question_id,
        question,
        header,
        context,
        options,
        allow_multiple,
        allow_freeform,
        suggest,
        kind,
    }
}
