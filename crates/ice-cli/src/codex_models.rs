//! Account-scoped ChatGPT Codex model discovery.
//!
//! The Codex backend lists the models the signed-in account may use at
//! `GET {base}/codex/models`. The result is cached in
//! `<agent dir>/codex-models.json`, and the built-in `openai-codex` catalog is
//! restricted to it, so models the account lacks are not offered and new
//! models appear on the next refresh (`/reload`, login) without a release.
//! Without a cache the built-in list is used.
//!
//! Response shape reference: the official Codex client's `models_cache.json`
//! (`slug`, `display_name`, `visibility`, `context_window`, `input_modalities`,
//! `supported_reasoning_levels`).

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::codex_backend;
use crate::config;
use crate::oauth_codex::CodexCredential;

const CACHE_FILE: &str = "codex-models.json";

/// A model the account may use, reduced to what the catalog needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DiscoveredModel {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) context_window: u64,
    pub(crate) supports_images: bool,
    pub(crate) supports_max_thinking: bool,
}

#[derive(Deserialize)]
struct ModelsResponse {
    models: Vec<RemoteModel>,
}

#[derive(Deserialize)]
struct RemoteModel {
    slug: String,
    display_name: Option<String>,
    #[serde(default)]
    visibility: String,
    context_window: Option<u64>,
    input_modalities: Option<Vec<String>>,
    #[serde(default)]
    supported_reasoning_levels: Vec<ReasoningLevel>,
}

#[derive(Deserialize)]
struct ReasoningLevel {
    effort: String,
}

impl RemoteModel {
    /// Only models the account's picker lists; `hide` entries are internal.
    fn into_listed(self) -> Option<DiscoveredModel> {
        (self.visibility == "list" && !self.slug.is_empty()).then(|| DiscoveredModel {
            name: self.display_name.unwrap_or_else(|| self.slug.clone()),
            id: self.slug,
            context_window: self.context_window.unwrap_or(272_000),
            supports_images: self
                .input_modalities
                .map_or(true, |modalities| modalities.iter().any(|m| m == "image")),
            supports_max_thinking: self
                .supported_reasoning_levels
                .iter()
                .any(|level| level.effort == "max"),
        })
    }
}

fn cache_path() -> Option<std::path::PathBuf> {
    config::agent_dir().ok().map(|dir| dir.join(CACHE_FILE))
}

/// The cached account model list, if any. A missing or unreadable cache means
/// "use the built-in catalog".
pub(crate) fn load() -> Option<Vec<DiscoveredModel>> {
    let bytes = std::fs::read(cache_path()?).ok()?;
    serde_json::from_slice::<Vec<DiscoveredModel>>(&bytes)
        .ok()
        .filter(|models| !models.is_empty())
}

fn store(models: &[DiscoveredModel]) -> Result<(), String> {
    let path = cache_path().ok_or("No config directory for the Codex model cache")?;
    let bytes = serde_json::to_vec_pretty(models).map_err(|error| error.to_string())?;
    config::atomic_write(&path, &bytes).map_err(|error| error.to_string())
}

/// Drop the cache; true if there was one.
fn clear() -> bool {
    cache_path().is_some_and(|path| std::fs::remove_file(path).is_ok())
}

fn models_url(base_url: &str) -> Result<Url, String> {
    let mut url = codex_backend::codex_url(base_url, "codex/models")?;
    url.query_pairs_mut()
        .append_pair("client_version", env!("CARGO_PKG_VERSION"));
    Ok(url)
}

async fn fetch(
    base_url: &str,
    credential: &CodexCredential,
    cancellation: &CancellationToken,
) -> Result<Vec<DiscoveredModel>, String> {
    let body = codex_backend::get(models_url(base_url)?, credential, cancellation).await?;
    let parsed: ModelsResponse =
        serde_json::from_slice(&body).map_err(|_| "Unrecognized model list response")?;
    Ok(parsed
        .models
        .into_iter()
        .filter_map(RemoteModel::into_listed)
        .collect())
}

/// Fetch the signed-in account's model list and cache it. Returns whether the
/// cached catalog source changed (and providers should be re-resolved). Without
/// a Codex login the cache is dropped. A failed or empty fetch leaves the
/// previous cache untouched so the catalog never shrinks to nothing because of
/// a transient error.
pub(crate) async fn refresh(
    base_url: &str,
    cancellation: CancellationToken,
) -> Result<bool, String> {
    let Some(credential) = codex_backend::signed_in(&cancellation).await? else {
        return Ok(clear());
    };
    let models = fetch(base_url, &credential, &cancellation).await?;
    if models.is_empty() {
        return Err("The account has no listed Codex models".into());
    }
    store(&models)?;
    Ok(true)
}

#[cfg(test)]
pub(crate) fn store_for_test(models: &[DiscoveredModel]) {
    store(models).unwrap();
}

#[cfg(test)]
mod tests;
