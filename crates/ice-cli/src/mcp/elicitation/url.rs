//! Browser consent and connection-local completion tracking.
use super::*;

type Browser = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
pub(super) struct UrlElicitation {
    ids: Mutex<HashMap<String, bool>>,
    notices: tokio::sync::broadcast::Sender<String>,
    browser: Browser,
}
struct Reservation<'a> {
    state: &'a UrlElicitation,
    id: &'a str,
    accepted: bool,
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            self.state.ids.lock().unwrap().remove(self.id);
        }
    }
}
impl UrlElicitation {
    pub fn new(notices: tokio::sync::broadcast::Sender<String>) -> Self {
        Self {
            ids: Default::default(),
            notices,
            browser: Arc::new(ice_tools::utils::open_browser::open_browser),
        }
    }
    pub fn complete(&self, server: &str, params: &Value) {
        let Some(id) = params["elicitationId"].as_str() else {
            return;
        };
        let mut ids = self.ids.lock().unwrap();
        if ids.get(id) != Some(&true) {
            return;
        }
        ids.remove(id);
        let _ = self.notices.send(format!(
            "MCP browser interaction for {} completed. You can retry the tool now.",
            clean(server)
        ));
    }
    pub async fn handle(
        &self,
        form: &Elicitation,
        params: &Value,
        signal: &CancellationToken,
    ) -> Result<Value, RpcError> {
        let message = params["message"]
            .as_str()
            .ok_or_else(|| invalid("Missing elicitation message"))?;
        let id = params["elicitationId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
            .ok_or_else(|| invalid("Invalid elicitationId (1–256 bytes required)"))?;
        let raw = params["url"]
            .as_str()
            .filter(|url| !url.chars().any(char::is_control) && url.trim() == *url)
            .ok_or_else(|| invalid("Invalid elicitation URL"))?;
        let url = reqwest::Url::parse(raw).map_err(|_| invalid("Invalid elicitation URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid(
                "Only HTTP(S) URLs without credentials are supported",
            ));
        }
        {
            let mut ids = self.ids.lock().unwrap();
            if ids.contains_key(id) || ids.len() >= 128 {
                return Err(invalid(
                    "Duplicate elicitationId or too many browser requests",
                ));
            }
            ids.insert(id.into(), false);
        }
        let mut reservation = Reservation {
            state: self,
            id,
            accepted: false,
        };
        let question = format!(
            "MCP browser request\nServer: {}\n\n{}\n\nHost: {}\nFull URL: {}\n\nOpen this URL in your browser?",
            clean(&form.server), clean(message), url.host_str().unwrap(), url.as_str()
        );
        match form
            .choose(&question, &["Open", "Decline", "Cancel"], signal)
            .await
        {
            Some(0) if !signal.is_cancelled() => {
                // No await between the final cancellation check, launch, and tracking.
                // A dropped future cannot leave an untracked browser launch. The ID
                // accepts completion before the launch, since the server may finish
                // while the browser is still starting; a failed launch releases it
                // through `reservation`.
                self.ids.lock().unwrap().insert(id.into(), true);
                if (self.browser)(url.as_str()).is_err() {
                    let _ = self.notices.send(format!(
                        "Could not open MCP browser request for {}.",
                        clean(&form.server)
                    ));
                    return Ok(json!({"action":"cancel"}));
                }
                reservation.accepted = true;
                Ok(json!({"action":"accept"}))
            }
            Some(1) => Ok(json!({"action":"decline"})),
            _ => Ok(json!({"action":"cancel"})),
        }
    }
}

#[cfg(test)]
mod tests;
