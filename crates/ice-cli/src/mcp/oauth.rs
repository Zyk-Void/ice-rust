//! OAuth for HTTP MCP servers (`auth: "oauth"`): protected-resource discovery
//! (RFC 9728), authorization-server metadata with the issuer check (RFC 8414),
//! dynamic client registration (RFC 7591) when no `clientId` is configured,
//! PKCE S256 authorization code with a loopback or pasted callback, refresh
//! tokens, and `client_credentials`. Ports pi-mcp-adapter `mcp-auth*.ts`.
//! Tokens live in `<agent dir>/mcp-auth.json` at mode 600 (D7), bound to the
//! server URL they were issued for.
use super::*;

use crate::oauth_codex::{self, CallbackListener, CallbackPages, OAuthError};

const FILE: &str = "mcp-auth.json";
/// Refresh this long before the reported expiry.
const EXPIRY_SKEW_SECS: i64 = 60;
const CALLBACK_WAIT: Duration = Duration::from_secs(5 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE: usize = 256 * 1024;
const PAGES: CallbackPages = CallbackPages {
    path: std::borrow::Cow::Borrowed("/callback"),
    success: "<!doctype html><html><head><meta charset=\"utf-8\"><title>Signed in</title></head><body><p>MCP sign-in complete. You can close this window and return to icers.</p></body></html>",
    error: "<!doctype html><html><head><meta charset=\"utf-8\"><title>Sign-in failed</title></head><body><p>MCP sign-in could not be completed. Return to icers and try again.</p></body></html>",
};

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConfig {
    #[serde(default)]
    pub grant_type: GrantType,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_name: Option<String>,
    pub auth_server_metadata_url: Option<String>,
    #[serde(default)]
    pub skip_issuer_metadata_validation: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    #[default]
    AuthorizationCode,
    ClientCredentials,
}

/// Stored credentials for one server. Secrets never appear in `Debug`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    /// The server URL the token was issued for; another URL never reuses it.
    pub url: String,
    pub token_endpoint: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
}
impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Record")
            .field("url", &self.url)
            .field("client_id", &self.client_id)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn read_all(agent_dir: &Path) -> BTreeMap<String, Record> {
    std::fs::read_to_string(agent_dir.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_all(agent_dir: &Path, records: &BTreeMap<String, Record>) -> Result<(), String> {
    std::fs::create_dir_all(agent_dir).map_err(|e| e.to_string())?;
    // NamedTempFile is created with mode 600 on Unix, then renamed into place.
    let mut file = tempfile::NamedTempFile::new_in(agent_dir).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut file, records).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(agent_dir.join(FILE))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn load(agent_dir: &Path, server: &str, url: &str) -> Option<Record> {
    read_all(agent_dir).remove(server).filter(|r| r.url == url)
}

/// Serializes this process's read-modify-write of the token file, so a refresh
/// for one server cannot overwrite a concurrent sign-in or logout for another.
static STORE: Mutex<()> = Mutex::new(());

fn save(agent_dir: &Path, server: &str, record: Record) -> Result<(), String> {
    let _store = STORE.lock().unwrap();
    let mut records = read_all(agent_dir);
    records.insert(server.to_owned(), record);
    write_all(agent_dir, &records)
}

/// Forget a server's credentials; `false` when there were none.
pub(crate) fn remove(agent_dir: &Path, server: &str) -> Result<bool, String> {
    let _store = STORE.lock().unwrap();
    let mut records = read_all(agent_dir);
    let removed = records.remove(server).is_some();
    if removed {
        write_all(agent_dir, &records)?;
    }
    Ok(removed)
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        // Metadata and token responses must come from the URL we asked.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// HTTPS, or plain HTTP to a loopback host (local servers and tests).
fn secure(url: &reqwest::Url) -> bool {
    url.scheme() == "https" || (url.scheme() == "http" && http::loopback(url))
}

async fn get_json(http: &reqwest::Client, url: &reqwest::Url) -> Result<Value, String> {
    if !secure(url) {
        return Err(format!("OAuth metadata URL must use https: {url}"));
    }
    let response = http
        .get(url.clone())
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    bounded_json(response).await
}

async fn bounded_json(response: reqwest::Response) -> Result<Value, String> {
    // Bounded while reading, so a hostile endpoint cannot exhaust memory.
    let bytes = http::read_body(response, MAX_RESPONSE, false)
        .await
        .map_err(|e| format!("OAuth {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("OAuth response is not JSON: {e}"))
}

/// `origin/.well-known/<suffix><path>` (RFC 8414 / 9728 path insertion).
fn well_known(url: &reqwest::Url, suffix: &str) -> Option<reqwest::Url> {
    let path = url.path().trim_end_matches('/');
    url.join(&format!("/.well-known/{suffix}{path}")).ok()
}

#[derive(Clone, Debug)]
struct Metadata {
    authorization_endpoint: Option<String>,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    challenge_methods: Option<Vec<String>>,
}

fn same_issuer(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

fn metadata(value: &Value, issuer: Option<&str>, skip_check: bool) -> Result<Metadata, String> {
    if let Some(issuer) = issuer.filter(|_| !skip_check) {
        let declared = value["issuer"].as_str().unwrap_or_default();
        if !same_issuer(declared, issuer) {
            return Err(format!(
                "OAuth issuer mismatch: metadata says {declared:?}, expected {issuer:?}"
            ));
        }
    }
    let endpoint = |key: &str| value[key].as_str().map(str::to_owned);
    Ok(Metadata {
        authorization_endpoint: endpoint("authorization_endpoint"),
        token_endpoint: endpoint("token_endpoint").ok_or("OAuth metadata has no token_endpoint")?,
        registration_endpoint: endpoint("registration_endpoint"),
        challenge_methods: value["code_challenge_methods_supported"]
            .as_array()
            .map(|methods| {
                methods
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_owned))
                    .collect()
            }),
    })
}

/// The `resource_metadata` URL from a 401's `WWW-Authenticate: Bearer ...`.
fn resource_metadata_hint(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = &header[at..];
    let value = match rest.strip_prefix('"') {
        Some(quoted) => &quoted[..quoted.find('"')?],
        None => rest.split([',', ' ']).next()?,
    };
    Some(value.to_owned())
}

/// Find the authorization server for `server_url` and the `resource` to request.
async fn discover(
    http: &reqwest::Client,
    server_url: &reqwest::Url,
    config: &OAuthConfig,
) -> Result<(Metadata, String), String> {
    let mut resource = server_url.as_str().to_owned();
    if let Some(url) = &config.auth_server_metadata_url {
        // Trusted configuration: authoritative, but the issuer must still match
        // the document's own origin unless the check is explicitly disabled.
        let url = reqwest::Url::parse(url).map_err(|e| format!("authServerMetadataUrl: {e}"))?;
        let value = get_json(http, &url).await?;
        let declared = value["issuer"].as_str().unwrap_or_default();
        // A well-known document names its issuer exactly (RFC 8414 path
        // insertion); any other document must at least share its origin.
        let well_known = [
            "/.well-known/oauth-authorization-server",
            "/.well-known/openid-configuration",
        ]
        .iter()
        .find_map(|prefix| url.path().strip_prefix(prefix));
        let matches = match well_known {
            Some(path) => same_issuer(
                declared,
                &format!("{}{path}", url.origin().ascii_serialization()),
            ),
            None => reqwest::Url::parse(declared).map(|u| u.origin()) == Ok(url.origin()),
        };
        if !config.skip_issuer_metadata_validation && !matches {
            return Err(format!(
                "OAuth issuer mismatch: {declared:?} is not served by {url}"
            ));
        }
        return Ok((metadata(&value, None, true)?, resource));
    }
    // The server's 401 may name its protected-resource metadata; else the well-known paths.
    let probe = http
        .post(server_url.clone())
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#)
        .send()
        .await
        .ok();
    let hint = probe
        .as_ref()
        .and_then(|r| r.headers().get("www-authenticate"))
        .and_then(|v| v.to_str().ok())
        .and_then(resource_metadata_hint)
        .and_then(|url| server_url.join(&url).ok());
    let candidates = hint.into_iter().chain(
        [
            well_known(server_url, "oauth-protected-resource"),
            server_url
                .join("/.well-known/oauth-protected-resource")
                .ok(),
        ]
        .into_iter()
        .flatten(),
    );
    let mut issuer = None;
    for url in candidates {
        if let Ok(value) = get_json(http, &url).await {
            issuer = value["authorization_servers"][0]
                .as_str()
                .map(str::to_owned);
            if let Some(declared) = value["resource"].as_str() {
                resource = declared.to_owned();
            }
            if issuer.is_some() {
                break;
            }
        }
    }
    // Servers without protected-resource metadata act as their own issuer
    // (the MCP 2025-03-26 rule).
    let issuer = match issuer {
        Some(issuer) => issuer,
        None => server_url.origin().ascii_serialization(),
    };
    let issuer_url = reqwest::Url::parse(&issuer).map_err(|e| format!("OAuth issuer: {e}"))?;
    let candidates = [
        well_known(&issuer_url, "oauth-authorization-server"),
        well_known(&issuer_url, "openid-configuration"),
        issuer_url
            .join(&format!(
                "{}/.well-known/openid-configuration",
                issuer_url.path().trim_end_matches('/')
            ))
            .ok(),
    ];
    let mut last = "no authorization server metadata found".to_string();
    for url in candidates.into_iter().flatten() {
        match get_json(http, &url).await {
            Ok(value) => {
                return Ok((
                    metadata(
                        &value,
                        Some(&issuer),
                        config.skip_issuer_metadata_validation,
                    )?,
                    resource,
                ))
            }
            Err(error) => last = error,
        }
    }
    Err(format!("OAuth discovery failed: {last}"))
}

/// Register a public client for `redirect_uri` (or a confidential one for
/// `client_credentials`), unless `clientId` is configured.
async fn client(
    http: &reqwest::Client,
    metadata: &Metadata,
    config: &OAuthConfig,
    redirect_uri: Option<&str>,
) -> Result<(String, Option<String>), String> {
    if let Some(id) = &config.client_id {
        let secret = config
            .client_secret
            .as_deref()
            .map(http::interpolate)
            .transpose()?;
        return Ok((id.clone(), secret));
    }
    let endpoint = metadata
        .registration_endpoint
        .as_deref()
        .ok_or("the server needs a pre-registered oauth.clientId (no registration endpoint)")?;
    let endpoint = reqwest::Url::parse(endpoint).map_err(|e| e.to_string())?;
    if !secure(&endpoint) {
        return Err("OAuth registration endpoint must use https".into());
    }
    let mut body = json!({
        "client_name": config.client_name.as_deref().unwrap_or("icers"),
        "grant_types": match config.grant_type {
            GrantType::AuthorizationCode => json!(["authorization_code", "refresh_token"]),
            GrantType::ClientCredentials => json!(["client_credentials"]),
        },
        "token_endpoint_auth_method": if redirect_uri.is_some() { "none" } else { "client_secret_post" },
    });
    if let Some(redirect) = redirect_uri {
        body["redirect_uris"] = json!([redirect]);
        body["response_types"] = json!(["code"]);
    }
    let response = http
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OAuth registration failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "OAuth registration failed: HTTP {}",
            response.status()
        ));
    }
    let value = bounded_json(response).await?;
    let id = value["client_id"]
        .as_str()
        .ok_or("OAuth registration returned no client_id")?;
    Ok((
        id.into(),
        value["client_secret"].as_str().map(str::to_owned),
    ))
}

/// Exchange a grant at the token endpoint and build the stored record.
#[allow(clippy::too_many_arguments)]
async fn token(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    mut form: Vec<(&str, String)>,
    url: &str,
    resource: Option<&str>,
    previous_refresh: Option<String>,
) -> Result<Record, String> {
    let endpoint = reqwest::Url::parse(token_endpoint).map_err(|e| e.to_string())?;
    if !secure(&endpoint) {
        return Err("OAuth token endpoint must use https".into());
    }
    form.push(("client_id", client_id.into()));
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.into()));
    }
    if let Some(resource) = resource {
        form.push(("resource", resource.into()));
    }
    let response = http
        .post(endpoint)
        .header("accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("OAuth token request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "OAuth token request failed: HTTP {}",
            response.status()
        ));
    }
    let value = bounded_json(response).await?;
    let access_token = value["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("OAuth token response has no access_token")?;
    Ok(Record {
        url: url.into(),
        token_endpoint: token_endpoint.into(),
        client_id: client_id.into(),
        client_secret: client_secret.map(str::to_owned),
        access_token: access_token.into(),
        // A refresh response may omit the refresh token; keep using the old one.
        refresh_token: value["refresh_token"]
            .as_str()
            .map(str::to_owned)
            .or(previous_refresh),
        // A hostile `expires_in` cannot overflow.
        expires_at: value["expires_in"]
            .as_i64()
            .map(|secs| now().saturating_add(secs.clamp(0, 10 * 365 * 24 * 3600))),
        resource: resource.map(str::to_owned),
    })
}

/// A usable access token for `server`, refreshing or (for `client_credentials`)
/// fetching one as needed. `Ok(None)` means the user has to sign in.
pub(crate) async fn access_token(
    agent_dir: &Path,
    server: &str,
    url: &str,
    config: &OAuthConfig,
) -> Result<Option<String>, String> {
    let record = load(agent_dir, server, url);
    if let Some(record) = &record {
        if record
            .expires_at
            .is_none_or(|at| at.saturating_sub(EXPIRY_SKEW_SECS) > now())
        {
            return Ok(Some(record.access_token.clone()));
        }
    }
    let http = http()?;
    if let Some(record) = record {
        if let Some(refresh) = record.refresh_token.clone() {
            let refreshed = token(
                &http,
                &record.token_endpoint,
                &record.client_id,
                record.client_secret.as_deref(),
                vec![
                    ("grant_type", "refresh_token".into()),
                    ("refresh_token", refresh.clone()),
                ],
                url,
                record.resource.as_deref(),
                Some(refresh),
            )
            .await;
            if let Ok(fresh) = refreshed {
                let token = fresh.access_token.clone();
                save(agent_dir, server, fresh)?;
                return Ok(Some(token));
            }
        }
    }
    if config.grant_type == GrantType::ClientCredentials {
        let server_url = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
        let (metadata, resource) = discover(&http, &server_url, config).await?;
        let (client_id, secret) = client(&http, &metadata, config, None).await?;
        let mut form = vec![("grant_type", "client_credentials".to_string())];
        if let Some(scope) = &config.scope {
            form.push(("scope", scope.clone()));
        }
        let record = token(
            &http,
            &metadata.token_endpoint,
            &client_id,
            secret.as_deref(),
            form,
            url,
            Some(&resource),
            None,
        )
        .await?;
        let token = record.access_token.clone();
        save(agent_dir, server, record)?;
        return Ok(Some(token));
    }
    Ok(None)
}

/// A sign-in in progress: what the code exchange needs, plus the listener.
pub(crate) struct Pending {
    pub url: String,
    pub authorize_url: String,
    state: String,
    verifier: String,
    redirect_uri: String,
    token_endpoint: String,
    client_id: String,
    client_secret: Option<String>,
    resource: String,
    pub listener: Option<CallbackListener>,
}

/// Prepare the browser step of a sign-in.
pub(crate) async fn start(url: &str, config: &OAuthConfig) -> Result<Pending, String> {
    if config.grant_type == GrantType::ClientCredentials {
        return Err("this server uses client_credentials; no browser sign-in is needed".into());
    }
    let http = http()?;
    let server_url = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let (metadata, resource) = discover(&http, &server_url, config).await?;
    if metadata
        .challenge_methods
        .as_ref()
        .is_some_and(|methods| !methods.iter().any(|m| m == "S256"))
    {
        return Err("the authorization server does not support PKCE S256".into());
    }
    let authorization = metadata
        .authorization_endpoint
        .clone()
        .ok_or("OAuth metadata has no authorization_endpoint")?;
    let authorization = reqwest::Url::parse(&authorization).map_err(|e| e.to_string())?;
    if !secure(&authorization) {
        return Err("OAuth authorization endpoint must use https".into());
    }
    // A loopback redirect gets a listener; anything else is completed by pasting.
    let (redirect_uri, listener) = match &config.redirect_uri {
        Some(configured) => {
            let parsed =
                reqwest::Url::parse(configured).map_err(|e| format!("oauth.redirectUri: {e}"))?;
            let loopback = parsed.scheme() == "http" && secure(&parsed);
            let listener = match (loopback, parsed.port()) {
                (true, Some(port)) => {
                    let pages = CallbackPages {
                        path: parsed.path().to_owned().into(),
                        ..PAGES
                    };
                    Some(
                        CallbackListener::bind_with(
                            std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                            pages,
                        )
                        .await
                        .map_err(|e| format!("cannot listen on {configured}: {e}"))?,
                    )
                }
                (true, None) => {
                    return Err("a loopback oauth.redirectUri needs an explicit port".into())
                }
                // The code would travel to a remote host in cleartext.
                _ if parsed.scheme() == "http" => {
                    return Err(
                        "oauth.redirectUri must use https unless it is a loopback address".into(),
                    )
                }
                _ => None,
            };
            (configured.clone(), listener)
        }
        None => {
            let listener =
                CallbackListener::bind_with(std::net::SocketAddr::from(([127, 0, 0, 1], 0)), PAGES)
                    .await
                    .map_err(|e| format!("cannot start the sign-in listener: {e}"))?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            (
                format!("http://127.0.0.1:{port}{}", PAGES.path),
                Some(listener),
            )
        }
    };
    let (client_id, client_secret) = client(&http, &metadata, config, Some(&redirect_uri)).await?;
    let verifier = oauth_codex::create_code_verifier().map_err(|e| e.to_string())?;
    let state = oauth_codex::create_state().map_err(|e| e.to_string())?;
    let mut authorize_url = authorization;
    {
        let mut query = authorize_url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair(
                "code_challenge",
                &oauth_codex::create_code_challenge(&verifier),
            )
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("resource", &resource);
        if let Some(scope) = &config.scope {
            query.append_pair("scope", scope);
        }
    }
    Ok(Pending {
        url: url.into(),
        authorize_url: authorize_url.into(),
        state,
        verifier,
        redirect_uri,
        token_endpoint: metadata.token_endpoint,
        client_id,
        client_secret,
        resource,
        listener,
    })
}

fn describe(error: OAuthError) -> String {
    match error {
        OAuthError::Cancelled => "MCP sign-in was cancelled".into(),
        OAuthError::CallbackTimeout => "MCP sign-in timed out waiting for the browser".into(),
        OAuthError::ProviderDenied => "MCP sign-in was denied".into(),
        OAuthError::StateMismatch => "the pasted URL belongs to a different sign-in".into(),
        OAuthError::MissingAuthorizationCode => "no authorization code found in the input".into(),
        _ => "MCP sign-in callback failed".into(),
    }
}

impl Pending {
    /// Wait for the browser to reach the loopback listener.
    pub(crate) async fn wait(
        listener: CallbackListener,
        state: String,
        cancel: CancellationToken,
    ) -> Result<String, String> {
        listener
            .wait_for_state(&state, cancel, CALLBACK_WAIT)
            .await
            .map(|code| code.as_str().to_owned())
            .map_err(describe)
    }
    pub(crate) fn state(&self) -> &str {
        &self.state
    }
    /// A pasted callback URL, query string, or bare code for this sign-in.
    pub(crate) fn parse(&self, input: &str) -> Result<String, String> {
        oauth_codex::parse_manual_code(input, &self.state)
            .map(|code| code.as_str().to_owned())
            .map_err(describe)
    }
    /// Exchange the code and store the tokens.
    pub(crate) async fn finish(
        self,
        agent_dir: &Path,
        server: &str,
        code: &str,
    ) -> Result<(), String> {
        let record = token(
            &http()?,
            &self.token_endpoint,
            &self.client_id,
            self.client_secret.as_deref(),
            vec![
                ("grant_type", "authorization_code".into()),
                ("code", code.into()),
                ("redirect_uri", self.redirect_uri.clone()),
                ("code_verifier", self.verifier.clone()),
            ],
            &self.url,
            Some(&self.resource),
            None,
        )
        .await?;
        save(agent_dir, server, record)
    }
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
