//! `icers auth` subcommand — persistent credential management. Mirrors the
//! Rust-relevant slice of the TS `packages/coding-agent/src/cli/auth-command.ts`
//! + `main.ts:runAuthCommand` + `core/auth-check.ts`.
//!
//! The host owns credential storage and standard login/check/logout/refresh
//! commands. Packages own provider policy and may supply OAuth implementations;
//! ChatGPT subscription login also has a built-in fallback.
//!
//! - `icers auth login`  — legacy API-key prompt (no echo, via `rpassword`) for
//!   the default provider and persist it to `~/.icers/auth.json`.
//!   Mirrors the upstream `/login` TUI's `type:"secret"` prompt + `modify`.
//! - `icers auth check`  — local-only probe of `auth.json`, models.json and the
//!   provider's declared policy; reports `ready`/`not_ready` (no network call — the upstream
//!   `--no-refresh` equivalent). `--json` emits a structured result.
//! - `icers auth logout` — drop the selected provider entry from `auth.json`
//!   (env vars are left untouched).
//!
//! Extension-owned OAuth takes precedence over the built-in Codex flow.

use std::io::{self, Write};
use std::sync::Arc;

use crate::config::{
    self, delete_credential, read_auth, upsert_credential, Credential, DEFAULT_PROVIDER_ID,
};
use crate::credentials::CredentialSource;
/// Exit code for a missing-credential `auth check` (mirrors upstream's
/// non-zero `auth check` when not ready).
const EXIT_NOT_READY: i32 = 1;
/// Exit code for an operational error (IO failure, bad args).
const EXIT_ERROR: i32 = 2;

async fn run_auth_store_io<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, config::ConfigError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| format!("credential worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

/// `icers auth <sub> [args]` entry. `args` is the slice *after* `auth` (i.e. the
/// subcommand + its flags). Returns the process exit code. Async to match the
/// `app::run` shape, though v1 does no async work here.
pub async fn run(args: &[String]) -> i32 {
    run_with_extensions(args, None).await
}

/// Run auth with an optional extension snapshot. The caller must keep the
/// matching extension session alive until this future completes.
pub async fn run_with_extensions(
    args: &[String],
    snapshot: Option<Arc<ice_extensions::RegistrySnapshot>>,
) -> i32 {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "login" => run_login(&args[1..], snapshot.as_ref()).await,
        "check" | "status" => run_check(&args[1..], snapshot.as_ref()).await,
        "refresh" => run_refresh(&args[1..], snapshot.as_ref()).await,
        "logout" => run_logout(&args[1..], snapshot.as_ref()).await,
        "--help" | "-h" | "help" | "" => {
            print_auth_help();
            0
        }
        other => {
            eprintln!("error: unknown auth subcommand \"{other}\"");
            eprintln!();
            print_auth_help();
            EXIT_ERROR
        }
    }
}

/// `icers auth login [--provider <id>]` — prompt for a key and persist it.
async fn run_login(
    args: &[String],
    snapshot: Option<&Arc<ice_extensions::RegistrySnapshot>>,
) -> i32 {
    let provider = parse_provider(args).unwrap_or(DEFAULT_PROVIDER_ID);
    if let Some(snapshot) = snapshot {
        if snapshot.oauth_provider(provider).is_some() {
            return run_oauth_login(provider, snapshot).await;
        }
    }
    if provider == crate::codex_auth::PROVIDER_ID {
        return run_codex_login().await;
    }
    // Compatibility API-key login. This path is provider-neutral; OAuth
    // behavior is supplied by the matching extension above.
    eprint!("Enter API key for {provider}: ");
    let key = match rpassword::read_password() {
        Ok(k) => k,
        Err(e) => {
            eprintln!();
            eprintln!("error: could not read the key from the terminal: {e}");
            return EXIT_ERROR;
        }
    };
    eprintln!();
    let key = key.trim();
    if key.is_empty() {
        eprintln!("error: an empty key was entered; nothing saved.");
        return EXIT_ERROR;
    }
    let cred = Credential::ApiKey {
        key: Some(key.to_string()),
        env: None,
    };
    let provider_id = provider.to_string();
    if let Err(e) = run_auth_store_io(move || upsert_credential(&provider_id, cred)).await {
        eprintln!("error: could not save credentials: {e}");
        return EXIT_ERROR;
    }
    let path = match config::auth_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("warn: credentials saved, but could not resolve the config path: {e}");
            return 0;
        }
    };
    println!("Credentials saved to {}", path.display());
    println!("Run `icers auth check --provider {provider}` to verify.");
    0
}

/// `icers auth check [--provider <id>] [--json]` — local readiness probe.
async fn run_check(
    args: &[String],
    snapshot: Option<&Arc<ice_extensions::RegistrySnapshot>>,
) -> i32 {
    let provider = parse_provider(args).unwrap_or(DEFAULT_PROVIDER_ID);
    let want_json = args.iter().any(|a| a == "--json");

    let source = match detect_credential(provider, snapshot) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: could not read authentication configuration: {error}");
            return EXIT_ERROR;
        }
    };
    let ready = source.is_present();
    let expires = config::read_oauth_token(provider)
        .ok()
        .flatten()
        .and_then(|credential| match credential {
            Credential::Oauth { expires, .. } if expires > 0 => Some(expires),
            _ => None,
        });
    if want_json {
        let json = serde_json::json!({
            "ready": ready,
            "provider": provider,
            "source": source.as_json_str(),
            "oauth_expires_at": expires,
        });
        println!("{json}");
    } else if ready {
        println!("ready ({})", source.as_json_str());
        if let Some(expires) = expires {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(expires);
            println!(
                "{provider}: OAuth, expires in {}h",
                expires.saturating_sub(now).max(0) / 3600
            );
        }
    } else {
        println!("not_ready — no credentials found.");
        eprintln!();
        eprintln!("Run `icers auth login --provider {provider}`, configure this provider's apiKey in models.json, or pass --api-key when starting icers.");
        let env = match snapshot.and_then(|snapshot| snapshot.provider_manifest(provider)) {
            Some(manifest) => manifest.api_key_env.as_deref(),
            None => crate::credentials::builtin_api_key_env(provider),
        };
        if let Some(env) = env {
            eprintln!("You can also set {env}.");
        }
    }
    if ready {
        0
    } else {
        EXIT_NOT_READY
    }
}

/// `icers auth refresh [--provider <id>]` — refresh a stored OAuth credential.
async fn run_refresh(
    args: &[String],
    snapshot: Option<&Arc<ice_extensions::RegistrySnapshot>>,
) -> i32 {
    let provider = parse_provider(args).unwrap_or(DEFAULT_PROVIDER_ID);
    if provider == crate::codex_auth::PROVIDER_ID
        && snapshot
            .and_then(|snapshot| snapshot.oauth_provider(provider))
            .is_none()
    {
        return match crate::codex_auth::refresh().await {
            Ok(()) => {
                println!("OAuth credentials refreshed for \"{provider}\".");
                0
            }
            Err(error) => {
                eprintln!("error: {error}");
                EXIT_ERROR
            }
        };
    }
    let Some(snapshot) = snapshot else {
        eprintln!("error: no OAuth extensions are loaded");
        return EXIT_ERROR;
    };
    let Some(Credential::Oauth {
        access,
        refresh,
        expires,
        ..
    }) = config::read_oauth_token(provider).ok().flatten()
    else {
        eprintln!("error: no stored OAuth credentials for \"{provider}\"");
        return EXIT_NOT_READY;
    };
    let response = match ice_extensions::request_oauth(
        snapshot,
        provider,
        "refresh",
        serde_json::json!({"credential": {"access": access, "refresh": refresh, "expires_at": expires}}),
    ) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: OAuth refresh failed: {error}");
            return EXIT_ERROR;
        }
    };
    let response = match ice_extensions::OAuthActionResponse::parse(response) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("error: invalid OAuth refresh response: {error}");
            return EXIT_ERROR;
        }
    };
    let Some(token) = (match response.credential() {
        Ok(token) => token,
        Err(error) => {
            eprintln!("error: invalid OAuth refresh credential: {error}");
            return EXIT_ERROR;
        }
    }) else {
        eprintln!("error: OAuth refresh returned no credential");
        return EXIT_ERROR;
    };
    let provider_id = provider.to_string();
    if let Err(error) = run_auth_store_io(move || {
        config::upsert_oauth_token(&provider_id, token.access, token.refresh, token.expires_at)
    })
    .await
    {
        eprintln!("error: could not save refreshed OAuth credentials: {error}");
        return EXIT_ERROR;
    }
    println!("OAuth credentials refreshed for \"{provider}\".");
    0
}

async fn run_logout(
    args: &[String],
    snapshot: Option<&Arc<ice_extensions::RegistrySnapshot>>,
) -> i32 {
    let provider = parse_provider(args).unwrap_or(DEFAULT_PROVIDER_ID);
    if provider == crate::codex_auth::PROVIDER_ID
        && snapshot
            .and_then(|snapshot| snapshot.oauth_provider(provider))
            .is_none()
    {
        return match run_auth_store_io(|| config::delete_credential(crate::codex_auth::PROVIDER_ID))
            .await
        {
            Ok(_) => {
                println!("Local Codex credentials removed.");
                0
            }
            Err(error) => {
                eprintln!("error: {error}");
                EXIT_ERROR
            }
        };
    }
    if let Ok(Some(Credential::Oauth {
        access, refresh, ..
    })) = config::read_oauth_token(provider)
    {
        let Some(snapshot) = snapshot else {
            eprintln!("error: cannot revoke OAuth credentials: no OAuth extensions are loaded");
            return EXIT_ERROR;
        };
        let response = match ice_extensions::request_oauth(
            snapshot,
            provider,
            "revoke",
            serde_json::json!({"credential": {"access": access, "refresh": refresh}}),
        ) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("error: OAuth revoke failed; local credentials were kept: {error}");
                return EXIT_ERROR;
            }
        };
        match ice_extensions::OAuthActionResponse::parse(response) {
            Ok(response) if response.kind == ice_extensions::OAuthResponseKind::Success => {}
            Ok(_) => {
                eprintln!(
                    "error: OAuth revoke did not return success; local credentials were kept"
                );
                return EXIT_ERROR;
            }
            Err(error) => {
                eprintln!(
                    "error: invalid OAuth revoke response; local credentials were kept: {error}"
                );
                return EXIT_ERROR;
            }
        }
        /*
         */
    }
    let provider_id = provider.to_string();
    match run_auth_store_io(move || delete_credential(&provider_id)).await {
        Ok(true) => {
            println!("Removed stored credentials for \"{provider}\".");
            0
        }
        Ok(false) => {
            println!("No stored credential for \"{provider}\" (nothing to do).");
            0
        }
        Err(e) => {
            eprintln!("error: could not remove credentials: {e}");
            EXIT_ERROR
        }
    }
}

/// Pull the `--provider <id>` value from a subcommand's args (defaults to
/// `None`). Mirrors the TS `--provider` handshake before it falls back to the
/// default.
async fn run_oauth_login(provider: &str, snapshot: &Arc<ice_extensions::RegistrySnapshot>) -> i32 {
    let mut input = serde_json::json!({});
    let mut action = "begin";
    loop {
        let raw = match ice_extensions::request_oauth(snapshot, provider, action, input) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("error: OAuth {action} failed: {error}");
                return EXIT_ERROR;
            }
        };
        let response = match ice_extensions::OAuthActionResponse::parse(raw) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("error: invalid OAuth response: {error}");
                return EXIT_ERROR;
            }
        };
        match response.kind {
            ice_extensions::OAuthResponseKind::Credential => {
                let Some(token) = response.credential().ok().flatten() else {
                    eprintln!("error: OAuth response contained no valid credential");
                    return EXIT_ERROR;
                };
                let provider_id = provider.to_string();
                if let Err(error) = run_auth_store_io(move || {
                    config::upsert_oauth_token(
                        &provider_id,
                        token.access,
                        token.refresh,
                        token.expires_at,
                    )
                })
                .await
                {
                    eprintln!("error: could not save OAuth credentials: {error}");
                    return EXIT_ERROR;
                }
                println!("OAuth credentials saved for \"{provider}\".");
                return 0;
            }
            ice_extensions::OAuthResponseKind::Error => {
                eprintln!("error: OAuth provider rejected the request");
                return EXIT_ERROR;
            }
            ice_extensions::OAuthResponseKind::Success => {
                eprintln!("error: OAuth login completed without credentials");
                return EXIT_ERROR;
            }
            ice_extensions::OAuthResponseKind::Interaction => {
                let display = response.payload.get("display").unwrap_or(&response.payload);
                if let Some(title) = display.get("title").and_then(serde_json::Value::as_str) {
                    println!("{title}");
                }
                if let Some(url) = display.get("url").and_then(serde_json::Value::as_str) {
                    println!("{url}");
                }
                if let Some(instructions) = display
                    .get("instructions")
                    .and_then(serde_json::Value::as_array)
                {
                    for instruction in instructions.iter().filter_map(serde_json::Value::as_str) {
                        println!("{instruction}");
                    }
                }
                eprint!("OAuth input: ");
                let _ = io::stderr().flush();
                let mut line = String::new();
                if io::stdin().read_line(&mut line).is_err() || line.trim().is_empty() {
                    eprintln!("error: empty OAuth input; nothing saved.");
                    return EXIT_ERROR;
                }
                input = serde_json::json!({"value": line.trim()});
                action = "exchange";
            }
        }
    }
}

async fn run_codex_login() -> i32 {
    let (url, flow) = match crate::oauth_codex::LoginFlow::begin().await {
        Ok(flow) => flow,
        Err(error) => {
            eprintln!("error: {error}");
            return EXIT_ERROR;
        }
    };
    println!("Open this URL to sign in:\n{url}");
    if let Err(error) = crate::codex_auth::open_browser(&url) {
        eprintln!("Could not open the browser ({error}); open the URL manually.");
    }
    println!("You may paste the redirect URL or authorization code here. Press Esc to cancel.");
    let flow_cancellation = tokio_util::sync::CancellationToken::new();
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let input_cancellation = flow_cancellation.clone();
    let input = tokio::task::spawn_blocking(move || codex_manual_input(sender, input_cancellation));
    let result = tokio::select! {
        result = flow.complete(receiver, flow_cancellation.clone()) => result.map_err(|error| error.to_string()),
        _ = tokio::signal::ctrl_c() => Err("Login cancelled".to_string()),
    };
    flow_cancellation.cancel();
    let _ = input.await;
    match result {
        Ok(credential) => match persist_codex_login_cancellable(credential, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        {
            Ok(()) => {
                println!("Codex OAuth credentials saved.");
                0
            }
            Err(error) if error == "Request aborted" => {
                eprintln!("error: Login cancelled");
                EXIT_ERROR
            }
            Err(error) => {
                eprintln!("error: could not save credentials: {error}");
                EXIT_ERROR
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            EXIT_ERROR
        }
    }
}

async fn persist_codex_login_cancellable<F>(
    credential: crate::oauth_codex::CodexCredential,
    cancellation: F,
) -> Result<(), String>
where
    F: std::future::Future<Output = ()>,
{
    let persistence_cancellation = tokio_util::sync::CancellationToken::new();
    let persistence =
        crate::codex_auth::persist_cancellable(credential, persistence_cancellation.clone());
    tokio::pin!(persistence);
    tokio::select! {
        result = &mut persistence => result,
        _ = cancellation => {
            persistence_cancellation.cancel();
            persistence.await
        }
    }
}

/// Poll terminal events so browser completion never leaves a blocked stdin reader.
fn codex_manual_input(
    sender: tokio::sync::mpsc::Sender<String>,
    cancellation: tokio_util::sync::CancellationToken,
) {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() {
        return;
    }
    struct RawMode;
    impl Drop for RawMode {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
    if crossterm::terminal::enable_raw_mode().is_err() {
        return;
    }
    let _raw = RawMode;
    let mut value = String::new();
    while !cancellation.is_cancelled() {
        match event::poll(std::time::Duration::from_millis(100)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return,
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind != event::KeyEventKind::Release => match key.code {
                KeyCode::Esc => {
                    cancellation.cancel();
                    return;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    cancellation.cancel();
                    return;
                }
                KeyCode::Enter if !value.trim().is_empty() => {
                    let _ = sender.blocking_send(value);
                    return;
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(character) => value.push(character),
                _ => {}
            },
            Ok(Event::Paste(text)) => value.push_str(&text),
            Err(_) => return,
            _ => {}
        }
    }
}

fn parse_provider(args: &[String]) -> Option<&str> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--provider" {
            if let Some(v) = iter.next() {
                return Some(v.as_str());
            }
        } else if let Some(rest) = a.strip_prefix("--provider=") {
            return Some(rest);
        }
    }
    None
}

/// Local readiness uses the same provider identity, precedence and package
/// policy as requests. It never exposes the resolved credential material.
fn detect_credential(
    provider: &str,
    snapshot: Option<&Arc<ice_extensions::RegistrySnapshot>>,
) -> Result<CredentialSource, config::ConfigError> {
    let store = read_auth()?;
    let models = config::load_models_config()?;
    let manifest = snapshot.and_then(|snapshot| snapshot.provider_manifest(provider));
    let resolved =
        crate::credentials::resolve(provider, models.providers.get(provider), &store, manifest);
    if resolved.source.is_present() {
        return Ok(resolved.source);
    }
    let manifests = snapshot.map_or_else(Vec::new, |snapshot| {
        snapshot
            .providers()
            .iter()
            .filter_map(|provider| snapshot.provider_manifest(&provider.provider_id).cloned())
            .collect()
    });
    let catalog = crate::provider::execution_catalog_with_manifests(&manifests)?;
    if catalog.iter().any(|model| {
        model.provider == provider && crate::credentials::has_header_auth(model.headers.as_ref())
    }) {
        return Ok(CredentialSource::Headers);
    }
    Ok(CredentialSource::Missing)
}

/// `icers auth --help`. Mirrors the TS auth-command usage but scoped to v1.
fn print_auth_help() {
    println!(
        "Usage: {name} auth <subcommand> [options]

Manage persisted credentials and inspect provider package authentication.

Subcommands:
  login   Prompt for an API key, or run an OAuth provider login.
  check   Report whether credentials are available (no network call).
  status  Alias for check; never prints token material.
  refresh Refresh built-in Codex or extension-owned OAuth credentials.
  logout  Remove saved credentials (extensions may also revoke them).

Options:
  --provider <id>   Provider id (default: anthropic)
  --extension <path>  Load a provider or OAuth extension explicitly.
  --no-extensions     Disable extension discovery and package auth declarations.
  --json            (check only) Emit a {{ready, provider, source}} JSON object

Environment:
  ANTHROPIC_API_KEY      Fallback API key (x-api-key) when no stored credential.
  ANTHROPIC_AUTH_TOKEN   Fallback bearer token (Authorization: Bearer).
  OPENAI_API_KEY         Fallback bearer token for the built-in openai identity.
  Other providers       Environment names come from installed package declarations.

Notes:
  `login --provider <id>` uses the registered OAuth extension when one exists;
  otherwise openai-codex uses built-in ChatGPT subscription OAuth, and other
  providers use a provider-neutral API-key compatibility prompt.
  Codex logout removes local credentials without revoking remote sessions.
  API-key providers may still use ~/.icers/agent/models.json or environment
  variables.
",
        name = crate::APP_NAME
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        auth_path, test_support::env_lock, upsert_credential, Credential, DEFAULT_PROVIDER_ID,
    };

    /// Scope `ICERS_CODING_AGENT_DIR` + the `ANTHROPIC_*` env vars to a temp dir
    /// for the duration of a test. Holds the shared env lock for its whole
    /// lifetime so it can't race with config/provider env-mutating tests.
    struct TempConfig {
        _guard: std::sync::MutexGuard<'static, ()>,
        _tmp: tempfile::TempDir,
        prev_dir: Option<std::ffi::OsString>,
        prev_key: Option<std::ffi::OsString>,
        prev_tok: Option<std::ffi::OsString>,
        prev_openai_key: Option<std::ffi::OsString>,
    }
    impl TempConfig {
        fn new() -> Self {
            let guard = env_lock().lock().unwrap();
            let prev_dir = std::env::var_os(crate::config::CONFIG_DIR_ENV);
            let prev_key = std::env::var_os(crate::provider::ANTHROPIC_API_KEY_ENV);
            let prev_tok = std::env::var_os(crate::provider::ANTHROPIC_AUTH_TOKEN_ENV);
            let prev_openai_key = std::env::var_os(crate::provider::OPENAI_API_KEY_ENV);
            let tmp = tempfile::TempDir::new().unwrap();
            std::env::set_var(crate::config::CONFIG_DIR_ENV, tmp.path());
            std::env::remove_var(crate::provider::ANTHROPIC_API_KEY_ENV);
            std::env::remove_var(crate::provider::ANTHROPIC_AUTH_TOKEN_ENV);
            std::env::remove_var(crate::provider::OPENAI_API_KEY_ENV);
            Self {
                _guard: guard,
                _tmp: tmp,
                prev_dir,
                prev_key,
                prev_tok,
                prev_openai_key,
            }
        }
    }
    impl Drop for TempConfig {
        fn drop(&mut self) {
            restore(crate::config::CONFIG_DIR_ENV, self.prev_dir.take());
            restore(crate::provider::ANTHROPIC_API_KEY_ENV, self.prev_key.take());
            restore(
                crate::provider::ANTHROPIC_AUTH_TOKEN_ENV,
                self.prev_tok.take(),
            );
            restore(
                crate::provider::OPENAI_API_KEY_ENV,
                self.prev_openai_key.take(),
            );
        }
    }
    fn restore(name: &str, prev: Option<std::ffi::OsString>) {
        match prev {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn standard_oauth_credential_response_is_parsed() {
        let response = ice_extensions::OAuthActionResponse::parse(serde_json::json!({
            "version": 1,
            "kind": "credential",
            "payload": {
                "access": "access",
                "refresh": "refresh",
                "expires_at": 123
            }
        }))
        .unwrap();
        let credential = response.credential().unwrap().unwrap();
        assert_eq!(credential.access, "access");
        assert_eq!(credential.refresh, "refresh");
        assert_eq!(credential.expires_at, 123);
    }

    #[tokio::test]
    async fn check_not_ready_with_no_credentials() {
        let _cfg = TempConfig::new();
        let code = run_check(&[], None).await;
        assert_eq!(code, EXIT_NOT_READY);
    }

    #[tokio::test]
    async fn check_ready_with_stored_credential() {
        let _cfg = TempConfig::new();
        upsert_credential(
            DEFAULT_PROVIDER_ID,
            Credential::ApiKey {
                key: Some("sk-stored".into()),
                env: None,
            },
        )
        .unwrap();
        let code = run_check(&[], None).await;
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn check_ready_with_oauth_credential() {
        let _cfg = TempConfig::new();
        crate::config::upsert_oauth_token(
            DEFAULT_PROVIDER_ID,
            "access".into(),
            "refresh".into(),
            0,
        )
        .unwrap();
        assert_eq!(run_check(&[], None).await, 0);
    }

    #[tokio::test]
    async fn codex_logout_without_extension_removes_local_credentials() {
        let _cfg = TempConfig::new();
        crate::config::upsert_oauth_token_with_account(
            "openai-codex",
            "access".into(),
            "refresh".into(),
            1,
            Some("account".into()),
        )
        .unwrap();
        let args = vec!["--provider".into(), "openai-codex".into()];
        assert_eq!(run_logout(&args, None).await, 0);
        assert!(crate::config::read_oauth_token("openai-codex")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn codex_login_cancellation_during_store_lock_wait_does_not_save_later() {
        let _cfg = TempConfig::new();
        let store_lock = crate::config::lock_auth_store().unwrap();
        let credential = crate::oauth_codex::CodexCredential::from_stored(
            "new-access".into(),
            "new-refresh".into(),
            4_000_000_000,
            Some("acct-new".into()),
        )
        .unwrap();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            persist_codex_login_cancellable(credential, async {
                let _ = cancel_rx.await;
            })
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        cancel_tx.send(()).unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("cancelled login remained blocked on the credential lock")
            .unwrap()
            .unwrap_err();
        assert_eq!(error, "Request aborted");

        drop(store_lock);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(crate::config::read_oauth_token("openai-codex")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn check_rejects_expired_oauth_credential() {
        let _cfg = TempConfig::new();
        crate::config::upsert_oauth_token(
            DEFAULT_PROVIDER_ID,
            "access".into(),
            "refresh".into(),
            1,
        )
        .unwrap();
        assert_eq!(run_check(&[], None).await, EXIT_NOT_READY);
    }

    #[tokio::test]
    async fn check_ready_with_env_api_key() {
        let _cfg = TempConfig::new();
        std::env::set_var(crate::provider::ANTHROPIC_API_KEY_ENV, "sk-env");
        let code = run_check(&[], None).await;
        assert_eq!(code, 0);
    }

    #[test]
    fn check_does_not_borrow_credentials_from_another_provider_or_protocol() {
        let _cfg = TempConfig::new();
        std::env::set_var(crate::provider::ANTHROPIC_API_KEY_ENV, "anthropic-key");
        std::env::set_var(crate::provider::OPENAI_API_KEY_ENV, "openai-key");
        std::fs::write(
            config::models_path().unwrap(),
            r#"{"providers":{
            "gateway":{"api":"openai-completions","models":[{"id":"model"}]},
            "another":{"api":"openai-completions","apiKey":"another-key","models":[{"id":"other"}]}
        }}"#,
        )
        .unwrap();
        for provider in ["gateway", "unknown", "openai-completions", "OpenAI"] {
            assert_eq!(
                detect_credential(provider, None).unwrap(),
                CredentialSource::Missing,
                "{provider}"
            );
        }
        // A gateway key also must not make the built-in OpenAI identity ready.
        std::env::remove_var(crate::provider::OPENAI_API_KEY_ENV);
        assert_eq!(
            detect_credential("openai", None).unwrap(),
            CredentialSource::Missing
        );
    }

    #[test]
    fn check_resolves_stored_references_and_rejects_empty_or_unresolved_values() {
        let _cfg = TempConfig::new();
        upsert_credential(
            "gateway",
            Credential::ApiKey {
                key: Some("$ICERS_TEST_STORED_KEY".into()),
                env: Some(std::collections::BTreeMap::from([(
                    "ICERS_TEST_STORED_KEY".into(),
                    "stored-key".into(),
                )])),
            },
        )
        .unwrap();
        assert_eq!(
            detect_credential("gateway", None).unwrap(),
            CredentialSource::StoredApiKey
        );
        for key in [Some("$ICERS_TEST_AUTH_MISSING_VARIABLE"), Some("   "), None] {
            upsert_credential(
                "gateway",
                Credential::ApiKey {
                    key: key.map(str::to_string),
                    env: None,
                },
            )
            .unwrap();
            assert_eq!(
                detect_credential("gateway", None).unwrap(),
                CredentialSource::Missing
            );
        }
    }

    #[tokio::test]
    async fn check_reports_config_errors_and_respects_resolved_header_auth() {
        let _cfg = TempConfig::new();
        std::fs::write(
            config::models_path().unwrap(),
            r#"{"providers":{"gateway":{
            "api":"openai-completions","models":[{"id":"model","headers":{"Authorization":""}}]
        }}}"#,
        )
        .unwrap();
        assert_eq!(
            detect_credential("gateway", None).unwrap(),
            CredentialSource::Missing
        );
        std::fs::write(config::models_path().unwrap(), r#"{"providers":{"gateway":{
            "api":"openai-completions","models":[{"id":"model","headers":{"Authorization":"Bearer key"}}]
        }}}"#).unwrap();
        assert_eq!(
            detect_credential("gateway", None).unwrap(),
            CredentialSource::Headers
        );
        std::fs::write(config::auth_path().unwrap(), "{invalid").unwrap();
        assert_eq!(run_check(&[], None).await, EXIT_ERROR);
    }

    #[tokio::test]
    async fn check_ready_with_openai_models_json_key() {
        let _cfg = TempConfig::new();
        std::fs::write(
            crate::config::models_path().unwrap(),
            r#"{"providers":{"gateway":{"api":"openai-completions","apiKey":"key","models":[{"id":"gpt-test"}]}}}"#,
        )
        .unwrap();
        let code = run_check(&["--provider".to_string(), "gateway".to_string()], None).await;
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn check_json_outputs_object() {
        let _cfg = TempConfig::new();
        // Capture stdout is awkward in unit tests; just assert the exit code +
        // that a stored cred flips `ready`.
        upsert_credential(
            DEFAULT_PROVIDER_ID,
            Credential::ApiKey {
                key: Some("sk-x".into()),
                env: None,
            },
        )
        .unwrap();
        let code = run_check(&["--json".to_string()], None).await;
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn logout_removes_stored_credential() {
        let _cfg = TempConfig::new();
        upsert_credential(
            DEFAULT_PROVIDER_ID,
            Credential::ApiKey {
                key: Some("sk".into()),
                env: None,
            },
        )
        .unwrap();
        assert!(auth_path().unwrap().exists());
        let code = run_logout(&[], None).await;
        assert_eq!(code, 0);
        // The entry should be gone → check is now not_ready.
        assert_eq!(run_check(&[], None).await, EXIT_NOT_READY);
    }

    #[tokio::test]
    async fn logout_when_empty_is_noop() {
        let _cfg = TempConfig::new();
        let code = run_logout(&[], None).await;
        assert_eq!(code, 0);
    }

    #[tokio::test]
    async fn unknown_auth_subcommand_errors() {
        let code = run(&["bogus".to_string()]).await;
        assert_eq!(code, EXIT_ERROR);
    }

    #[tokio::test]
    async fn auth_help_exits_zero() {
        let code = run(&[]).await;
        assert_eq!(code, 0);
        let code = run(&["--help".to_string()]).await;
        assert_eq!(code, 0);
    }

    #[test]
    fn parse_provider_handles_both_forms() {
        assert_eq!(parse_provider(&[]), None);
        assert_eq!(
            parse_provider(&["--provider".to_string(), "anthropic".to_string()]),
            Some("anthropic")
        );
        assert_eq!(
            parse_provider(&["--provider=custom".to_string()]),
            Some("custom")
        );
        // A trailing flag with no value doesn't panic.
        assert_eq!(parse_provider(&["--provider".to_string()]), None);
    }
}
