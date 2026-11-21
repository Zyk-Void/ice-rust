//! Read-only, project-scoped discovery for ACP session/list.

use super::{INTERNAL_ERROR, INVALID_PARAMS};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ice_harness::session::jsonl::{parse_header, parse_mutation, JsonlSessionMetadata};
use ice_harness::session::types::SessionMutation;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

const PAGE_SIZE: usize = 50;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    cwd: PathBuf,
    modified_at: i64,
    path: String,
}

pub(super) async fn sessions(
    params: &Value,
    configured_dir: Option<&Path>,
) -> Result<Value, (i64, String)> {
    let cwd = params["cwd"]
        .as_str()
        .map(PathBuf::from)
        .filter(|cwd| cwd.is_absolute() && cwd.is_dir())
        .ok_or((
            INVALID_PARAMS,
            "`cwd` must be an existing absolute directory".to_string(),
        ))?;
    let cwd = cwd
        .canonicalize()
        .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
    let cursor = match params.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(encoded)) if encoded.len() <= 16 * 1024 => {
            let cursor = URL_SAFE_NO_PAD
                .decode(encoded)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Cursor>(&bytes).ok())
                .filter(|cursor| {
                    cursor.version == 1 && cursor.cwd == cwd && !cursor.path.is_empty()
                })
                .ok_or((
                    INVALID_PARAMS,
                    "invalid session-list cursor for this working directory".to_string(),
                ))?;
            Some(cursor)
        }
        Some(_) => {
            return Err((
                INVALID_PARAMS,
                "`cursor` must be a session-list cursor".to_string(),
            ))
        }
    };
    let dir = configured_dir
        .map(|dir| cwd.join(dir))
        .unwrap_or_else(|| crate::session::default_session_dir(&cwd));
    let mut metadata =
        crate::session::discover_session_metadata_in_dir(&cwd.to_string_lossy(), &dir)
            .await
            .map_err(|error| (INTERNAL_ERROR, error.to_string()))?
            .sessions;
    // A configured root can contain several projects and native imports.
    metadata.retain(|meta| Path::new(&meta.cwd).canonicalize().ok().as_ref() == Some(&cwd));
    metadata.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    if let Some(cursor) = cursor {
        metadata.retain(|meta| {
            meta.modified_at < cursor.modified_at
                || (meta.modified_at == cursor.modified_at && meta.path > cursor.path)
        });
    }
    let has_more = metadata.len() > PAGE_SIZE;
    metadata.truncate(PAGE_SIZE);
    let next_cursor = if has_more {
        let last = metadata.last().expect("a full page has a final entry");
        let cursor = Cursor {
            version: 1,
            cwd: cwd.clone(),
            modified_at: last.modified_at,
            path: last.path.clone(),
        };
        Some(URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&cursor).map_err(|error| (INTERNAL_ERROR, error.to_string()))?,
        ))
    } else {
        None
    };
    let sessions = tokio::task::spawn_blocking(move || {
        metadata
            .into_iter()
            .map(|meta| {
                let title = saved_title(&meta);
                let mut info = json!({
                    "sessionId": meta.id,
                    "cwd": cwd,
                    "updatedAt": crate::blackhole::js::iso_from_ms(meta.modified_at),
                });
                if let Some(title) = title {
                    info["title"] = json!(title);
                }
                info
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| (INTERNAL_ERROR, error.to_string()))?;
    let mut result = json!({"sessions": sessions});
    if let Some(cursor) = next_cursor {
        result["nextCursor"] = json!(cursor);
    }
    Ok(result)
}

fn saved_title(meta: &JsonlSessionMetadata) -> Option<String> {
    let file = std::fs::File::open(&meta.path).ok()?;
    let length = file.metadata().ok()?.len();
    // Never claim native writer ownership or follow an actively growing tail.
    let mut reader = std::io::BufReader::new(file.take(length));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let header = parse_header(line.trim_end()).ok()?;
    if header.id != meta.id || header.cwd != meta.cwd {
        return None;
    }
    let mut name = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        // A writer may have only partially appended its last mutation.
        if !line.ends_with('\n') {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        if let SessionMutation::FactName { name: next, .. } =
            parse_mutation(line.trim_end()).ok()?
        {
            name = next;
        }
    }
    name.filter(|name| !name.trim().is_empty())
}
