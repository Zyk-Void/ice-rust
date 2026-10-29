//! Remote model discovery. [`refresh`] only reads; [`persist`] saves the
//! discovered IDs into the provider's `models.json` entry so they survive
//! restarts and show up in every catalog consumer.
use std::collections::HashSet;
use std::time::Duration;

use ice_ai::{types::Api, Model};
use ice_extensions::ProviderManifest;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION},
    Url,
};
use serde::Deserialize;

use crate::config::{self, AuthStore, ProviderConfig};
use crate::credentials::{self, CredentialSource};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Discover new IDs for one provider, excluding all existing catalog entries.
/// Returned descriptors contain unknown model metadata and no secret headers;
/// the caller must retain existing entries and apply provider-level runtime
/// configuration when binding new models. Supply the execution catalog, the
/// selected provider's config, and its matching package manifest if any.
/// No network request is made when `offline` is true.
pub(crate) async fn refresh(
    provider: &str,
    catalog: &[Model],
    provider_config: Option<&ProviderConfig>,
    auth_store: &AuthStore,
    manifest: Option<&ProviderManifest>,
    offline: bool,
) -> Result<Vec<Model>, String> {
    if offline {
        return Err("Model discovery is unavailable offline".into());
    }
    if manifest.is_some_and(|m| m.id != provider) {
        return Err("Provider manifest does not match the selected provider".into());
    }
    let template = catalog
        .iter()
        .find(|m| m.provider == provider)
        .ok_or_else(|| "Selected provider has no configured model or endpoint".to_string())?;
    if !matches!(template.api, Api::OpenaiCompletions | Api::OpenaiResponses) {
        return Err("Model discovery is unsupported for this provider protocol".into());
    }
    let endpoint = models_endpoint(&template.base_url)?;
    let credential = credentials::resolve(provider, provider_config, auth_store, manifest);
    let mut headers = HeaderMap::new();
    if let Some(configured) = provider_config.and_then(|c| c.headers.as_ref()) {
        for (name, value) in configured {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "Invalid configured header name")?;
            let value = config::resolve_config_value_uncached(value, None)
                .ok_or("Could not resolve a configured header")?;
            let value =
                HeaderValue::from_str(&value).map_err(|_| "Invalid configured header value")?;
            headers.insert(name, value);
        }
    }
    let header_auth = ["authorization", "x-api-key", "cf-aig-authorization"]
        .iter()
        .any(|name| {
            headers
                .get(*name)
                .is_some_and(|value| !value.as_bytes().is_empty())
        });
    if !header_auth {
        if let Some(key) = credential.key {
            let value = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| "Invalid provider credential")?;
            headers.insert(AUTHORIZATION, value);
        } else if matches!(credential.source, CredentialSource::Missing) {
            return Err("No credential is available for the selected provider".into());
        }
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Could not initialize model discovery client")?;
    let mut response = client
        .get(endpoint)
        .headers(headers)
        .send()
        .await
        .map_err(|_| "Model discovery request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "Model discovery returned HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err("Model discovery response is too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read model discovery response")?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("Model discovery response is too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    let ids = parse_ids(&body)?;
    Ok(merge_models(provider, catalog, template, ids))
}

/// Append newly discovered models to the provider's `models.json` entry.
/// Existing definitions are never modified. A provider that is not declared in
/// `models.json` (built-in or package) is left alone and returns `Ok(0)`.
pub(crate) fn persist(provider: &str, discovered: &[Model]) -> Result<usize, String> {
    let mut cfg = config::load_models_config().map_err(|e| e.to_string())?;
    let Some(entry) = cfg.providers.get_mut(provider) else {
        return Ok(0);
    };
    let mut added = 0;
    for model in discovered {
        if entry
            .models
            .iter()
            .any(|existing| existing.id.eq_ignore_ascii_case(&model.id))
        {
            continue;
        }
        entry.models.push(config::ModelDefinition {
            id: model.id.clone(),
            name: None,
            base_url: None,
            reasoning: None,
            thinking_level_map: None,
            context_window: None,
            max_tokens: Some(model.max_tokens as u64),
            cost: None,
            input: None,
            headers: None,
            compat: None,
        });
        added += 1;
    }
    if added > 0 {
        config::save_models_config(&cfg).map_err(|e| e.to_string())?;
    }
    Ok(added)
}

fn models_endpoint(base: &str) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|_| "Invalid provider endpoint")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Provider endpoint must not contain credentials, query, or fragment".into());
    }
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("Provider endpoint must use HTTPS (HTTP is allowed only for loopback)".into());
    }
    let path = url.path().trim_end_matches('/');
    if path.ends_with("/chat/completions")
        || path.ends_with("/responses")
        || path.ends_with("/models")
    {
        return Err("Provider endpoint must be a base URL, not an API operation".into());
    }
    let last = path.rsplit('/').next().unwrap_or("");
    let version = last.strip_prefix('v').is_some_and(|s| {
        let n = s.chars().take_while(|c| c.is_ascii_digit()).count();
        n > 0
            && s.chars()
                .skip(n)
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    let path = if version {
        format!("{path}/models")
    } else {
        format!("{path}/v1/models")
    };
    url.set_path(&path);
    Ok(url)
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelId>,
}
#[derive(Deserialize)]
struct ModelId {
    id: String,
}

fn parse_ids(body: &[u8]) -> Result<Vec<String>, String> {
    let list: ModelList =
        serde_json::from_slice(body).map_err(|_| "Invalid model discovery response")?;
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for item in list.data {
        if item.id.trim().is_empty() || item.id.chars().any(char::is_control) {
            return Err("Invalid model ID in discovery response".into());
        }
        if seen.insert(item.id.clone()) {
            ids.push(item.id);
        }
    }
    if ids.is_empty() {
        return Err("Model discovery returned no models".into());
    }
    Ok(ids)
}

fn merge_models(
    provider: &str,
    catalog: &[Model],
    template: &Model,
    ids: Vec<String>,
) -> Vec<Model> {
    let mut result = Vec::new();
    let mut known: HashSet<String> = catalog
        .iter()
        .filter(|m| m.provider == provider)
        .map(|m| m.id.clone())
        .collect();
    for id in ids {
        if known.insert(id.clone()) {
            let mut model =
                Model::new(&id, &id, template.api.clone(), provider, &template.base_url);
            // A model listing does not provide capability metadata. Reuse the
            // configured output limit so requests do not default to one token.
            model.max_tokens = if template.max_tokens == 0 {
                4096
            } else {
                template.max_tokens
            };
            // Headers are deliberately excluded, including any resolved secrets.
            model.compat = template.compat.clone();
            result.push(model);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        Model::new(
            id,
            id,
            Api::OpenaiCompletions,
            "test",
            "https://example.com/v1",
        )
    }

    #[test]
    fn endpoints_follow_versioned_base_and_reject_unsafe_urls() {
        assert_eq!(
            models_endpoint("https://example.com").unwrap().as_str(),
            "https://example.com/v1/models"
        );
        assert_eq!(
            models_endpoint("https://example.com/proxy/v2/")
                .unwrap()
                .as_str(),
            "https://example.com/proxy/v2/models"
        );
        assert!(models_endpoint("http://127.0.0.1:1234/v1").is_ok());
        for url in [
            "http://example.com/v1",
            "https://user:secret@example.com",
            "https://example.com/v1?key=secret",
            "https://example.com/v1/chat/completions",
        ] {
            assert!(models_endpoint(url).is_err());
        }
    }

    #[test]
    fn persist_appends_only_new_ids_to_configured_providers() {
        let _lock = config::test_support::env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var_os(config::CONFIG_DIR_ENV);
        std::env::set_var(config::CONFIG_DIR_ENV, dir.path());
        std::fs::write(
            config::models_path().unwrap(),
            r#"{"providers":{"test":{"api":"openai-completions","baseUrl":"https://e.example/v1","models":[{"id":"known","name":"Keep"}]}}}"#,
        )
        .unwrap();
        let added = persist("test", &[model("KNOWN"), model("new")]).unwrap();
        assert_eq!(added, 1);
        assert_eq!(persist("builtin", &[model("x")]).unwrap(), 0);
        let cfg = config::load_models_config().unwrap();
        let ids: Vec<_> = cfg.providers["test"]
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, ["known", "new"]);
        assert_eq!(
            cfg.providers["test"].models[0].name.as_deref(),
            Some("Keep")
        );
        match prev {
            Some(v) => std::env::set_var(config::CONFIG_DIR_ENV, v),
            None => std::env::remove_var(config::CONFIG_DIR_ENV),
        }
    }

    #[test]
    fn parser_rejects_bad_empty_and_deduplicates() {
        assert_eq!(
            parse_ids(br#"{"data":[{"id":"one"},{"id":"one"},{"id":"two"}]}"#).unwrap(),
            ["one", "two"]
        );
        for body in [
            &b"{}"[..],
            &b"{\"data\":[]}"[..],
            &b"{\"data\":[{\"id\":\" \"}]}"[..],
        ] {
            assert!(parse_ids(body).is_err());
        }
    }

    #[test]
    fn merge_preserves_local_metadata_and_other_provider_is_excluded() {
        let mut existing = model("known");
        existing.context_window = 12345;
        existing.name = "Custom name".into();
        let other = Model::new(
            "other",
            "other",
            Api::OpenaiResponses,
            "other",
            "https://example.com",
        );
        let result = merge_models(
            "test",
            &[existing.clone(), other],
            &existing,
            vec!["known".into(), "new".into()],
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "new");
        assert_eq!(result[0].context_window, 0);
        assert_eq!(result[0].max_tokens, 4096);
        assert!(result[0].headers.is_none());
        assert_eq!(existing.name, "Custom name");
        assert_eq!(existing.context_window, 12345);
    }

    #[tokio::test]
    async fn discovers_from_local_server_with_authentication() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
            assert!(request.starts_with("get /v1/models "));
            assert!(request.contains("authorization: bearer test-secret"));
            let body = r#"{"data":[{"id":"known"},{"id":"new"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let mut known = model("known");
        known.base_url = format!("http://{addr}/v1");
        known.context_window = 42;
        let config: ProviderConfig =
            serde_json::from_value(serde_json::json!({"apiKey": "test-secret"})).unwrap();
        let result = refresh(
            "test",
            &[known],
            Some(&config),
            &AuthStore::new(),
            None,
            false,
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "new");
        assert!(result[0].headers.is_none());
    }

    #[tokio::test]
    async fn offline_and_unsupported_fail_before_network() {
        let mut unsupported = model("one");
        unsupported.api = Api::AnthropicMessages;
        assert!(refresh(
            "test",
            &[unsupported.clone()],
            None,
            &AuthStore::new(),
            None,
            true
        )
        .await
        .unwrap_err()
        .contains("offline"));
        assert!(
            refresh("test", &[unsupported], None, &AuthStore::new(), None, false)
                .await
                .unwrap_err()
                .contains("unsupported")
        );
    }
}
