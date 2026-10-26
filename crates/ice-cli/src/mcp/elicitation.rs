//! Server-initiated questions use the session's existing dialog mailbox.
use super::elicitation_schema::{Field, Schema};
use super::*;
use serde_json::Map;

pub(super) fn register(handlers: Handlers, server: &str, ui: UiDialogMailbox) -> Handlers {
    let state = Arc::new(Elicitation {
        server: server.into(),
        ui,
    });
    handlers.request(
        "elicitation/create",
        json!({"form":{}}),
        Arc::new(move |params, signal| {
            let state = state.clone();
            Box::pin(async move { state.handle(params, &signal).await })
        }),
    )
}
struct Elicitation {
    server: String,
    ui: UiDialogMailbox,
}
#[cfg(test)]
mod tests;

fn invalid(message: impl Into<String>) -> RpcError {
    RpcError {
        code: -32602,
        message: message.into(),
    }
}
fn clean(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}
impl Elicitation {
    async fn ask(&self, mut ui: Value, signal: &CancellationToken) -> Option<Value> {
        if signal.is_cancelled() {
            return None;
        }
        ui["header"] = json!(format!("MCP input: {}", self.server));
        let id = format!("mcp-elicit-{}", uuid::Uuid::new_v4());
        self.ui.handle(json!({"requestId":id,"ui":ui})).ok()?;
        // Reclaims dialogs even when dispatch drops this future on cancellation/EOF.
        let _guard = ApprovalGuard {
            ui: &self.ui,
            id: id.clone(),
        };
        loop {
            let state = self.ui.poll(&id).ok()?;
            match state["status"].as_str() {
                Some("answered") => return Some(state["answer"].clone()),
                Some("cancelled") => return None,
                _ => {}
            }
            tokio::select! {
                biased;
                _ = signal.cancelled() => return None,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    }
    async fn choose(
        &self,
        question: &str,
        options: &[&str],
        signal: &CancellationToken,
    ) -> Option<usize> {
        let answer = self.ask(json!({"kind":"selector","question":clean(question),"options":options.iter().map(|title| json!({"title":clean(title)})).collect::<Vec<_>>(),"allowFreeform":false}), signal).await?;
        let selected = answer["values"][0].as_str()?;
        options.iter().position(|option| clean(option) == selected)
    }
    async fn handle(&self, params: Value, signal: &CancellationToken) -> Result<Value, RpcError> {
        if params.get("mode").is_some_and(|mode| mode != "form") {
            return Err(invalid("Only form elicitation is supported"));
        }
        let message = params["message"]
            .as_str()
            .ok_or_else(|| invalid("Missing elicitation message"))?;
        let schema = Schema::parse(&params["requestedSchema"]).map_err(invalid)?;
        match self
            .choose(message, &["Continue", "Decline", "Cancel"], signal)
            .await
        {
            Some(0) => {}
            Some(1) => return Ok(json!({"action":"decline"})),
            _ => return Ok(json!({"action":"cancel"})),
        }
        let mut values = Map::new();
        for field in &schema.fields {
            if !self.collect(field, &mut values, signal).await {
                return Ok(json!({"action":"cancel"}));
            }
        }
        loop {
            let review = schema
                .fields
                .iter()
                .map(|field| {
                    format!(
                        "{}: {}",
                        clean(&field.title),
                        values
                            .get(&field.name)
                            .map(|v| clean(&v.to_string()))
                            .unwrap_or_else(|| "(omitted)".into())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            match self
                .choose(
                    &format!("Review input for {}\n\n{review}", self.server),
                    &["Submit", "Edit", "Decline", "Cancel"],
                    signal,
                )
                .await
            {
                Some(0) if !signal.is_cancelled() => {
                    return Ok(json!({"action":"accept","content":values}))
                }
                Some(1) if !schema.fields.is_empty() => {
                    let labels = schema
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| format!("{}. {}", index + 1, clean(&field.title)))
                        .collect::<Vec<_>>();
                    let options = labels.iter().map(String::as_str).collect::<Vec<_>>();
                    let Some(index) = self
                        .choose("Choose a field to edit", &options, signal)
                        .await
                    else {
                        return Ok(json!({"action":"cancel"}));
                    };
                    if !self
                        .collect(&schema.fields[index], &mut values, signal)
                        .await
                    {
                        return Ok(json!({"action":"cancel"}));
                    }
                }
                Some(1) => continue,
                Some(2) => return Ok(json!({"action":"decline"})),
                _ => return Ok(json!({"action":"cancel"})),
            }
        }
    }
    async fn collect(
        &self,
        field: &Field,
        values: &mut Map<String, Value>,
        signal: &CancellationToken,
    ) -> bool {
        let question = format!(
            "{}{}\n{}",
            clean(&field.title),
            if field.required { " (required)" } else { "" },
            clean(&field.description)
        );
        let mut error = String::new();
        loop {
            let mut actions = vec!["Enter value"];
            if field.default.is_some() {
                actions.push("Use default");
            }
            if !field.required {
                actions.push("Omit");
            }
            let Some(action) = self
                .choose(&format!("{question}\n{error}"), &actions, signal)
                .await
            else {
                return false;
            };
            if actions[action] == "Omit" {
                values.remove(&field.name);
                return true;
            }
            let value = if actions[action] == "Use default" {
                field.default.clone().unwrap()
            } else if field.options.is_empty() {
                let Some(answer) = self.ask(json!({"kind":"input","question":question,"context":clean(&error),"suggest":values.get(&field.name).and_then(Value::as_str).unwrap_or("")}),signal).await else { return false; };
                let Some(text) = answer["text"].as_str().or_else(|| answer["value"].as_str())
                else {
                    return false;
                };
                json!(text)
            } else {
                // Prefixes keep even identical/sanitized labels unambiguous. The
                // server's raw enum value is never reconstructed from display text.
                let labels = field
                    .options
                    .iter()
                    .enumerate()
                    .map(|(index, (_, title))| format!("{}. {}", index + 1, clean(title)))
                    .collect::<Vec<_>>();
                let Some(answer) = self.ask(json!({"kind":"selector","question":question,"options":labels.iter().map(|title| json!({"title":title})).collect::<Vec<_>>(),"allowMultiple":field.multiple,"allowFreeform":false}),signal).await else { return false; };
                let Some(selected) = answer["values"].as_array() else {
                    return false;
                };
                let selected = selected
                    .iter()
                    .map(|label| {
                        labels
                            .iter()
                            .position(|v| label.as_str() == Some(v))
                            .map(|index| field.options[index].0.clone())
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(selected) = selected else {
                    return false;
                };
                if field.multiple {
                    Value::Array(selected)
                } else if selected.len() == 1 {
                    selected[0].clone()
                } else {
                    return false;
                }
            };
            match field.validate(value) {
                Ok(value) => {
                    values.insert(field.name.clone(), value);
                    return true;
                }
                Err(message) => error = format!("Invalid value: {}", clean(&message)),
            }
        }
    }
}
