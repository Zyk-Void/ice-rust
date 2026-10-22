//! Private execution state. Public model snapshots never carry request headers
//! or credentials; requests recover their configuration by model identity here.
use super::*;

#[derive(Clone, Default)]
pub(super) enum RequestAuth {
    #[default]
    None,
    ApiKey(String),
    Headers(BTreeMap<String, String>),
    Oauth(Arc<crate::codex_auth::CodexToken>),
}

pub(super) struct ExecutionBinding {
    provider_id: String,
    models: Vec<Model>,
    descriptors: Vec<Model>,
    auth: RequestAuth,
}

impl std::fmt::Debug for ExecutionBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionBinding")
            .field("provider", &self.provider_id)
            .field("models", &self.models.len())
            .field("auth", &"[redacted]")
            .finish()
    }
}

pub(super) fn descriptor(model: &Model) -> Model {
    let mut model = model.clone();
    model.headers = None;
    model
}

impl ExecutionBinding {
    pub fn new(provider_id: &str, models: Vec<Model>, auth: RequestAuth) -> Self {
        Self {
            provider_id: provider_id.into(),
            descriptors: models.iter().map(descriptor).collect(),
            models,
            auth,
        }
    }

    pub fn models(&self) -> &[Model] {
        &self.descriptors
    }

    pub fn prepare(
        &self,
        requested: &Model,
        options: &SimpleStreamOptions,
    ) -> Option<(Model, SimpleStreamOptions)> {
        // The caller may select a published model, but cannot substitute an
        // endpoint or headers to redirect an already-bound credential.
        let model = self
            .models
            .iter()
            .find(|model| {
                requested.provider == self.provider_id
                    && model.provider == requested.provider
                    && model.api == requested.api
                    && model.id.eq_ignore_ascii_case(&requested.id)
            })?
            .clone();
        let mut options = options.clone();
        if options.api_key.is_none() {
            match &self.auth {
                RequestAuth::None | RequestAuth::Oauth(_) => {}
                RequestAuth::ApiKey(key) => options.api_key = Some(key.clone()),
                RequestAuth::Headers(headers) => {
                    let mut merged = headers.clone();
                    for (name, value) in options.headers.take().unwrap_or_default() {
                        merged.retain(|known, _| !known.eq_ignore_ascii_case(&name));
                        merged.insert(name, value);
                    }
                    options.headers = Some(merged);
                }
            }
        }
        Some((model, options))
    }

    pub async fn prepare_request(
        &self,
        requested: &Model,
        options: &SimpleStreamOptions,
    ) -> Result<Option<(Model, SimpleStreamOptions)>, String> {
        let Some((model, mut options)) = self.prepare(requested, options) else {
            return Ok(None);
        };
        if let RequestAuth::Oauth(token) = &self.auth {
            let credential = if let Some(access) = options.api_key.as_deref() {
                // An override must bring its own account identity, not reuse the
                // stored token's account header.
                crate::oauth_codex::CodexCredential::from_stored(
                    access.to_string(),
                    "explicit-override".into(),
                    i64::MAX,
                    None,
                )
                .map_err(|error| error.to_string())?
            } else {
                ice_ai::providers::openai_codex_responses::validate_oauth_base_url(&model.base_url)
                    .map_err(str::to_string)?;
                token.fresh(options.signal.clone()).await?
            };
            options.api_key = Some(credential.access_token().to_string());
            let headers = options.headers.get_or_insert_with(BTreeMap::new);
            headers.retain(|name, _| {
                !name.eq_ignore_ascii_case("chatgpt-account-id")
                    && !name.eq_ignore_ascii_case("authorization")
                    && !name.eq_ignore_ascii_case("x-api-key")
            });
            headers.insert(
                "chatgpt-account-id".into(),
                credential.account_id().as_str().into(),
            );
        }
        Ok(Some((model, options)))
    }

    #[cfg(test)]
    pub fn request_model(&self, requested: &Model) -> Model {
        let (mut model, options) = self
            .prepare(requested, &SimpleStreamOptions::default())
            .unwrap();
        if let Some(headers) = &options.headers {
            merge_auth_headers(&mut model, headers);
        }
        if model.api != ice_ai::Api::AnthropicMessages {
            if let Some(key) = options.api_key {
                merge_auth_headers(
                    &mut model,
                    &BTreeMap::from([("authorization".into(), format!("Bearer {key}"))]),
                );
            }
        }
        model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Credential;

    fn access_token(account: &str) -> String {
        use base64::Engine;
        let payload =
            serde_json::json!({"https://api.openai.com/auth": {"chatgpt_account_id":account}});
        format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
        )
    }

    fn binding() -> (ExecutionBinding, Model) {
        let model = Model::new(
            "codex",
            "Codex",
            ice_ai::Api::OpenaiCodexResponses,
            "openai-codex",
            "http://127.0.0.1:1",
        );
        let token = crate::codex_auth::CodexToken::new(Credential::Oauth {
            access: access_token("stored-account"),
            refresh: "refresh".into(),
            expires: i64::MAX,
            account_id: Some("stored-account".into()),
        })
        .unwrap();
        (
            ExecutionBinding::new(
                "openai-codex",
                vec![model.clone()],
                RequestAuth::Oauth(token),
            ),
            model,
        )
    }

    #[tokio::test]
    async fn stored_oauth_rejects_custom_endpoint_before_store_access() {
        let (binding, model) = binding();
        let error = binding
            .prepare_request(&model, &SimpleStreamOptions::default())
            .await
            .unwrap_err();
        assert!(error.contains("Stored Codex OAuth credentials require"));
    }

    #[test]
    fn oauth_endpoint_validation_rejects_untrusted_destinations() {
        use ice_ai::providers::openai_codex_responses::validate_oauth_base_url;
        for url in [
            "http://chatgpt.com/backend-api",
            "https://example.com/backend-api",
            "https://chatgpt.com.evil.test/backend-api",
            "https://chatgpt.com:444/backend-api",
            "https://user@chatgpt.com/backend-api",
            "https://chatgpt.com/other",
        ] {
            assert!(validate_oauth_base_url(url).is_err(), "{url}");
        }
        assert!(validate_oauth_base_url("https://chatgpt.com/backend-api").is_ok());
        assert!(validate_oauth_base_url("https://chatgpt.com/backend-api/codex").is_ok());
    }

    #[tokio::test]
    async fn invalid_model_is_rejected_before_accessing_stored_auth() {
        let (binding, mut model) = binding();
        model.id = "unbound".into();
        assert!(binding
            .prepare_request(&model, &SimpleStreamOptions::default())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn explicit_access_override_uses_its_own_account_without_store_access() {
        let (binding, model) = binding();
        let access = access_token("override-account");
        let options = SimpleStreamOptions {
            api_key: Some(access.clone()),
            headers: Some(BTreeMap::from([(
                "CHATGPT-ACCOUNT-ID".into(),
                "stale".into(),
            )])),
            ..Default::default()
        };
        let (_, options) = binding
            .prepare_request(&model, &options)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(options.api_key.as_deref(), Some(access.as_str()));
        let headers = options.headers.unwrap();
        assert_eq!(headers["chatgpt-account-id"], "override-account");
        assert!(!headers.contains_key("CHATGPT-ACCOUNT-ID"));
    }
}
