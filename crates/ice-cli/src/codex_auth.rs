//! Built-in Codex credential lifecycle. Protocol details live in `oauth_codex`.
use crate::config::{self, AuthStoreTransaction, Credential};
use crate::oauth_codex::{CodexCredential, CodexOAuthClient};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub(crate) const PROVIDER_ID: &str = "openai-codex";
const REFRESH_WINDOW_SECONDS: i64 = 5 * 60;
const LOGIN_HINT: &str = "Run icers auth login --provider openai-codex.";

/// Shared only within a private execution binding; credentials never enter models.
pub(crate) struct CodexToken {
    credential: Mutex<CodexCredential>,
    client: CodexOAuthClient,
}

impl std::fmt::Debug for CodexToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexToken")
            .field("credential", &"[redacted]")
            .finish()
    }
}

fn now_seconds() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock precedes the Unix epoch".to_string())?
        .as_secs()
        .try_into()
        .map_err(|_| "System clock is out of range".to_string())
}

fn from_stored(credential: Credential) -> Result<CodexCredential, String> {
    match credential {
        Credential::Oauth {
            access,
            refresh,
            expires,
            account_id,
        } => CodexCredential::from_stored(access, refresh, expires, account_id)
            .map_err(|error| format!("Invalid Codex credentials: {error}. {LOGIN_HINT}")),
        _ => Err(format!("No stored Codex OAuth credentials. {LOGIN_HINT}")),
    }
}

fn stored(credential: &CodexCredential) -> Credential {
    Credential::Oauth {
        access: credential.access_token().to_string(),
        refresh: credential.refresh_token().to_string(),
        expires: credential.expires_at_epoch_seconds(),
        account_id: Some(credential.account_id().as_str().to_string()),
    }
}

fn same_credential(left: &CodexCredential, right: &CodexCredential) -> bool {
    left.access_token() == right.access_token()
        && left.refresh_token() == right.refresh_token()
        && left.expires_at_epoch_seconds() == right.expires_at_epoch_seconds()
        && left.account_id().as_str() == right.account_id().as_str()
}

fn request_aborted() -> String {
    "Request aborted".into()
}

/// Cancel the operation if its async caller drops the future. The blocking
/// worker checks this token before entering the atomic store write.
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Nonblocking file-lock attempts keep cancellation from orphaning a lock waiter.
async fn transaction(cancellation: &CancellationToken) -> Result<AuthStoreTransaction, String> {
    let wait = async {
        loop {
            let attempt = tokio::task::spawn_blocking(config::try_lock_auth_store)
                .await
                .map_err(|_| "Credential lock worker failed".to_string())?
                .map_err(|error| format!("Could not lock credentials: {error}"))?;
            if let Some(transaction) = attempt {
                return Ok(transaction);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        _ = cancellation.cancelled() => Err("Request aborted".into()),
        result = tokio::time::timeout(Duration::from_secs(20), wait) =>
            result.map_err(|_| "Timed out waiting for credential storage".to_string())?,
    }
}

/// Apply one read-modify-write while retaining the supplied store transaction.
/// Cancellation is honored through the final check before `write_auth`; after
/// that boundary the atomic write is allowed to finish and its result is
/// authoritative, so callers never receive cancellation while a detached save
/// can still publish afterward.
async fn write_auth_transaction(
    transaction: AuthStoreTransaction,
    provider_id: &'static str,
    credential: Credential,
    cancellation: CancellationToken,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        if cancellation.is_cancelled() {
            return Err(request_aborted());
        }
        let mut store = transaction.read_auth().map_err(|error| error.to_string())?;
        if cancellation.is_cancelled() {
            return Err(request_aborted());
        }
        store.insert(provider_id.to_string(), credential);
        if cancellation.is_cancelled() {
            return Err(request_aborted());
        }
        transaction
            .write_auth(&store)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "Credential writer failed".to_string())?
}

/// Do credential-file IO away from Tokio's async workers. The transaction is
/// moved through the blocking task so its lock remains held while the read is
/// in progress and is released if the async caller is cancelled.
async fn read_auth(
    transaction: AuthStoreTransaction,
    cancellation: &CancellationToken,
) -> Result<(AuthStoreTransaction, config::AuthStore), String> {
    let worker_cancellation = cancellation.clone();
    let worker = tokio::task::spawn_blocking(move || {
        if worker_cancellation.is_cancelled() {
            return (transaction, Err(request_aborted()));
        }
        let result = transaction.read_auth().map_err(|error| error.to_string());
        (transaction, result)
    });

    tokio::select! {
        _ = cancellation.cancelled() => Err(request_aborted()),
        result = worker => {
            let (transaction, store) = result
                .map_err(|_| "Credential reader failed".to_string())?;
            Ok((transaction, store?))
        }
    }
}

/// Persist a refreshed entry under the already-held store transaction. If
/// cancellation arrives before the worker starts the write, it leaves the
/// store unchanged. A write already in progress is allowed to finish atomically
/// while retaining the lock, so a waiting logout runs after it and removes the
/// entry instead of racing with a stale refresh.
async fn persist_refresh(
    transaction: AuthStoreTransaction,
    credential: Credential,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let operation_cancellation = cancellation.child_token();
    let _cancel_on_drop = CancelOnDrop(operation_cancellation.clone());
    write_auth_transaction(transaction, PROVIDER_ID, credential, operation_cancellation).await
}

impl CodexToken {
    pub(crate) fn new(credential: Credential) -> Result<Arc<Self>, String> {
        let client = CodexOAuthClient::new().map_err(|error| error.to_string())?;
        Self::with_client(credential, client)
    }

    fn with_client(credential: Credential, client: CodexOAuthClient) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            credential: Mutex::new(from_stored(credential)?),
            client,
        }))
    }

    pub(crate) async fn fresh(
        &self,
        cancellation: CancellationToken,
    ) -> Result<CodexCredential, String> {
        self.fresh_inner(cancellation, false).await
    }

    async fn fresh_inner(
        &self,
        cancellation: CancellationToken,
        force: bool,
    ) -> Result<CodexCredential, String> {
        let mut cached = tokio::select! {
            _ = cancellation.cancelled() => return Err(request_aborted()),
            guard = self.credential.lock() => guard,
        };
        let previous = cached.clone();
        // Reread under the shared store lock even for an unexpired cached token:
        // logout or another process's rotated refresh token must win.
        let transaction = transaction(&cancellation).await?;
        let (transaction, mut store) = read_auth(transaction, &cancellation).await?;
        let current = store
            .remove(PROVIDER_ID)
            .ok_or_else(|| format!("Codex credentials were removed. {LOGIN_HINT}"))?;
        let current = from_stored(current)?;
        let now = now_seconds()?;
        let another_holder_refreshed = !same_credential(&previous, &current);
        // An explicit refresh still refreshes unchanged credentials, but if
        // another holder rotated the store while this caller waited, use that
        // result instead of immediately refreshing the newly rotated token.
        if current.expires_at_epoch_seconds().saturating_sub(now) > REFRESH_WINDOW_SECONDS
            && (!force || another_holder_refreshed)
        {
            *cached = current.clone();
            return Ok(current);
        }
        // Keep the cross-process lock through refresh: releasing it here lets
        // another process consume the same rotating refresh token. A later
        // compare-and-swap cannot undo that remote token rotation. Cancellation
        // and the HTTP timeout bound this wait; store I/O stays off the runtime.
        let refreshed = match self
            .client
            .refresh_token(current.refresh_token(), now, cancellation.clone())
            .await
        {
            Ok(refreshed) => refreshed,
            Err(_) if cancellation.is_cancelled() => return Err(request_aborted()),
            Err(error) => {
                return Err(format!("Codex token refresh failed: {error}. {LOGIN_HINT}"));
            }
        };
        if cancellation.is_cancelled() {
            return Err(request_aborted());
        }
        persist_refresh(transaction, stored(&refreshed), &cancellation)
            .await
            .map_err(|error| {
                if error == "Request aborted" {
                    request_aborted()
                } else {
                    format!("Could not save refreshed credentials: {error}")
                }
            })?;
        *cached = refreshed.clone();
        Ok(refreshed)
    }
}

pub(crate) async fn refresh() -> Result<(), String> {
    let credential = tokio::task::spawn_blocking(|| config::read_oauth_token(PROVIDER_ID))
        .await
        .map_err(|_| "Credential reader failed".to_string())?
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("No stored Codex credentials. {LOGIN_HINT}"))?;
    CodexToken::new(credential)?
        .fresh_inner(CancellationToken::new(), true)
        .await?;
    Ok(())
}

/// Persist a Codex login result with cancellable store-lock acquisition.
/// Cancellation is honored through the final check before `write_auth`; after
/// that write starts, this future waits for its atomic result, including when
/// cancellation races with the write.
pub(crate) async fn persist_cancellable(
    credential: CodexCredential,
    cancellation: CancellationToken,
) -> Result<(), String> {
    let operation_cancellation = cancellation.child_token();
    let _cancel_on_drop = CancelOnDrop(operation_cancellation.clone());
    let transaction = transaction(&operation_cancellation).await?;
    write_auth_transaction(
        transaction,
        PROVIDER_ID,
        stored(&credential),
        operation_cancellation,
    )
    .await
}

pub(crate) fn open_browser(url: &str) -> Result<(), String> {
    let mut command: Command;
    #[cfg(target_os = "windows")]
    {
        // Invoke the URL handler directly. Passing it through `cmd /c start`
        // lets query-string ampersands become shell metacharacters.
        command = Command::new("explorer.exe");
        command.arg(url);
    }
    #[cfg(target_os = "macos")]
    {
        command = Command::new("open");
        command.arg(url);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        command = Command::new("xdg-open");
        command.arg(url);
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = url;
        return Err("Opening a browser is unsupported on this platform".into());
    }

    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    reap_browser_child(child)
}

/// Reap the short-lived platform URL-handler process without blocking the
/// caller (which may be the TUI event loop).
fn reap_browser_child(child: Child) -> Result<(), String> {
    let child = Arc::new(StdMutex::new(Some(child)));
    let reaper_child = Arc::clone(&child);
    match std::thread::Builder::new()
        .name("icers-browser-reaper".into())
        .spawn(move || {
            let child = reaper_child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(mut child) = child {
                let _ = child.wait();
            }
        }) {
        Ok(_reaper) => Ok(()),
        Err(error) => {
            // Thread creation is unusual, but do not leave an un-reaped child
            // behind if the process cannot start a reaper.
            if let Some(mut child) = child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
            Err(format!("Could not monitor browser launcher: {error}"))
        }
    }
}

#[cfg(test)]
#[path = "codex_auth/tests.rs"]
mod tests;
