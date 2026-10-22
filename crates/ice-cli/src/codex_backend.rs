//! Authenticated reads from the ChatGPT backend for the signed-in Codex
//! account. Model discovery and usage reporting share this transport.

use std::time::Duration;

use reqwest::Url;
use tokio_util::sync::CancellationToken;

use crate::codex_auth::{CodexToken, PROVIDER_ID};
use crate::config::{self, Credential};
use crate::oauth_codex::CodexCredential;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// The stored Codex login with a fresh access token, or `None` when the user
/// has not signed in with ChatGPT.
pub(crate) async fn signed_in(
    cancellation: &CancellationToken,
) -> Result<Option<CodexCredential>, String> {
    let stored = config::read_auth().map_err(|error| error.to_string())?;
    let Some(credential @ Credential::Oauth { .. }) = stored.get(PROVIDER_ID) else {
        return Ok(None);
    };
    CodexToken::new(credential.clone())?
        .fresh(cancellation.clone())
        .await
        .map(Some)
}

/// `GET url` as the account, returning the bounded response body.
pub(crate) async fn get(
    url: Url,
    credential: &CodexCredential,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, String> {
    let request = reqwest::Client::new()
        .get(url)
        .bearer_auth(credential.access_token())
        .header("chatgpt-account-id", credential.account_id().as_str())
        .header("originator", "ice")
        .header("accept", "application/json")
        .timeout(REQUEST_TIMEOUT);
    let mut response = tokio::select! {
        _ = cancellation.cancelled() => return Err("Request aborted".into()),
        response = request.send() => response.map_err(|error| format!("http error: {error}"))?,
    };
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("Response is too large".into());
    }
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err("Request aborted".into()),
            chunk = response.chunk() => chunk.map_err(|error| format!("http error: {error}"))?,
        };
        let Some(chunk) = chunk else { break };
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            return Err("Response is too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn rejects_oversized_chunked_body_before_the_response_finishes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            for _ in 0..=MAX_RESPONSE_BYTES / chunk.len() {
                if socket.write_all(b"10000\r\n").await.is_err()
                    || socket.write_all(&chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            // No terminating chunk: the client must reject without waiting for EOF.
            std::future::pending::<()>().await;
        });
        let credential = CodexCredential::from_stored(
            "access-token".into(),
            "refresh-token".into(),
            i64::MAX,
            Some("account-id".into()),
        )
        .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            get(url, &credential, &CancellationToken::new()),
        )
        .await;
        server.abort();
        assert_eq!(result.unwrap().unwrap_err(), "Response is too large");
    }
}

/// `{base}/codex/{path}`-style URL builder shared by the callers: accepts a
/// base with or without a trailing `/codex`.
pub(crate) fn codex_url(base_url: &str, path: &str) -> Result<Url, String> {
    let base = base_url.trim().trim_end_matches('/');
    let base = base.strip_suffix("/codex").unwrap_or(base);
    Url::parse(&format!("{base}/{path}"))
        .map_err(|error| format!("Invalid Codex base URL: {error}"))
}
