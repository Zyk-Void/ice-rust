//! Host binding for typed decision endpoints declared under `models.json`
//! `decisionProviders`.
//!
//! Features call [`decision_client`] with a provider ID and then
//! [`DecisionClient::decide`] with a request of the matching protocol. The
//! endpoint, protocol, and key come only from that entry: there is no
//! environment fallback to another provider's key and no chat fallback.
//!
//! ```json
//! { "decisionProviders": { "typesafe": {
//!     "protocol": "typesafe-systemone",
//!     "baseUrl": "https://api.typesafe.ai/v1",
//!     "apiKey": "$TYPESAFE_API_KEY" } } }
//! ```

use ice_ai::decision::client::{DecisionClient, DecisionEndpoint, DecisionError, DecisionProtocol};
use thiserror::Error;

use crate::config::{self, AuthStore, ConfigError, Credential, ModelsConfig};

/// Why a decision provider could not be bound. Never contains key material.
#[derive(Debug, Error)]
pub enum DecisionConfigError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("no decision provider {0:?} in models.json decisionProviders")]
    UnknownProvider(String),
    #[error("decision provider {provider:?} has unsupported protocol {protocol:?}")]
    UnsupportedProtocol { provider: String, protocol: String },
    #[error(
        "decision provider {0:?} has no API key (set apiKey in models.json or store one in auth.json)"
    )]
    MissingCredential(String),
    #[error("decision provider {provider:?}: {source}")]
    Client {
        provider: String,
        #[source]
        source: DecisionError,
    },
}

/// Bind the named decision provider from the current `models.json`,
/// `auth.json`, and `httpIdleTimeout` setting.
pub fn decision_client(provider_id: &str) -> Result<DecisionClient, DecisionConfigError> {
    let models = config::load_models_config()?;
    let store = config::read_auth()?;
    let idle_timeout_ms = crate::settings::load_settings()?.http_idle_timeout_ms();
    client_from_config(provider_id, &models, &store, idle_timeout_ms)
}

pub(crate) fn client_from_config(
    provider_id: &str,
    models: &ModelsConfig,
    store: &AuthStore,
    idle_timeout_ms: Option<i64>,
) -> Result<DecisionClient, DecisionConfigError> {
    let entry = models
        .decision_providers
        .get(provider_id)
        .ok_or_else(|| DecisionConfigError::UnknownProvider(provider_id.into()))?;
    let protocol = DecisionProtocol::parse(&entry.protocol).ok_or_else(|| {
        DecisionConfigError::UnsupportedProtocol {
            provider: provider_id.into(),
            protocol: entry.protocol.clone(),
        }
    })?;
    let api_key = resolve_api_key(provider_id, entry.api_key.as_deref(), store)
        .ok_or_else(|| DecisionConfigError::MissingCredential(provider_id.into()))?;
    let headers = entry
        .headers
        .as_ref()
        .map(|headers| config::resolve_headers(headers, None))
        .unwrap_or_default();
    let endpoint = DecisionEndpoint {
        protocol,
        base_url: entry.base_url.clone(),
        api_key,
        headers,
    };
    DecisionClient::new(endpoint, idle_timeout_ms).map_err(|source| DecisionConfigError::Client {
        provider: provider_id.into(),
        source,
    })
}

/// Same precedence as chat API keys: a stored `auth.json` API key, then the
/// entry's `apiKey`. OAuth tokens are never reused for decision endpoints.
fn resolve_api_key(
    provider_id: &str,
    configured: Option<&str>,
    store: &AuthStore,
) -> Option<String> {
    let nonblank = |key: String| (!key.trim().is_empty()).then_some(key);
    if let Some(Credential::ApiKey {
        key: Some(key),
        env,
    }) = store.get(provider_id)
    {
        if let Some(key) = config::resolve_config_value(key, env.as_ref()).and_then(nonblank) {
            return Some(key);
        }
    }
    configured
        .and_then(|raw| config::resolve_config_value(raw, None))
        .and_then(nonblank)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(json: &str) -> ModelsConfig {
        serde_json::from_str(json).unwrap()
    }

    const CONFIG: &str = r#"{
        "providers": {},
        "decisionProviders": {
            "typesafe": {
                "protocol": "typesafe-systemone",
                "baseUrl": "https://api.typesafe.ai/v1",
                "apiKey": "ts-from-models",
                "headers": { "x-team": "triage" }
            },
            "typo": { "protocol": "typesafe", "baseUrl": "https://x.test/v1", "apiKey": "k" },
            "keyless": { "protocol": "inception-decisions", "baseUrl": "https://x.test/v1" }
        }
    }"#;

    #[test]
    fn binds_protocol_endpoint_and_models_json_key() {
        let models = models(CONFIG);
        let client = client_from_config("typesafe", &models, &AuthStore::new(), None).unwrap();
        assert_eq!(client.protocol(), DecisionProtocol::TypeSafeSystemOne);
        assert_eq!(client.url(), "https://api.typesafe.ai/v1/systemone");
        assert!(!format!("{client:?}").contains("ts-from-models"));
    }

    #[test]
    fn auth_json_key_wins_and_oauth_is_ignored() {
        let models = models(CONFIG);
        let store = AuthStore::from([(
            "keyless".to_string(),
            Credential::ApiKey {
                key: Some("stored".into()),
                env: None,
            },
        )]);
        assert!(client_from_config("keyless", &models, &store, None).is_ok());

        let oauth: AuthStore = serde_json::from_value(serde_json::json!({
            "keyless": { "type": "oauth", "access": "token", "refresh": "", "expires": 0 }
        }))
        .unwrap();
        assert!(matches!(
            client_from_config("keyless", &models, &oauth, None),
            Err(DecisionConfigError::MissingCredential(_))
        ));
    }

    #[test]
    fn unknown_provider_bad_protocol_and_chat_entries_are_not_decision_providers() {
        let models = models(
            r#"{
            "providers": { "openai": { "api": "openai-responses", "apiKey": "k" } },
            "decisionProviders": {
                "typo": { "protocol": "typesafe", "baseUrl": "https://x.test/v1", "apiKey": "k" }
            }
        }"#,
        );
        let store = AuthStore::new();
        assert!(matches!(
            client_from_config("openai", &models, &store, None),
            Err(DecisionConfigError::UnknownProvider(_))
        ));
        let error = client_from_config("typo", &models, &store, None).unwrap_err();
        assert!(matches!(
            error,
            DecisionConfigError::UnsupportedProtocol { .. }
        ));
        assert!(!error.to_string().contains("\"k\""));
    }

    #[test]
    fn decision_providers_round_trip_and_are_omitted_when_empty() {
        let parsed = models(CONFIG);
        let written = serde_json::to_value(&parsed).unwrap();
        assert_eq!(
            written["decisionProviders"]["typesafe"]["baseUrl"],
            "https://api.typesafe.ai/v1"
        );
        let empty = serde_json::to_value(models(r#"{"providers":{}}"#)).unwrap();
        assert!(empty.get("decisionProviders").is_none());
    }
}
