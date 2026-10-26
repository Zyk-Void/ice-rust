//! Bounds MCP results before they reach model context and the session file.
//! Mirrors pi-mcp-adapter `mcp-output-guard.ts` and the blob handling in
//! `tool-registrar.ts`; spill files live in one private directory per session.
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine;
use ice_agent::types::TextContentOrImage;
use ice_tools::{
    format_size, split_lines_for_counting, truncate_head, TruncationOptions, DEFAULT_MAX_BYTES,
    DEFAULT_MAX_LINES,
};
use serde_json::{json, Value};

const DEFAULT_DETAILS_MAX_BYTES: usize = 16 * 1024;
const MAX_BLOB_BYTES: usize = 10 * 1024 * 1024;
const MAX_SESSION_BLOB_BYTES: usize = 100 * 1024 * 1024;
/// Bounds metadata from empty or tiny blobs.
const MAX_SESSION_BLOB_FILES: usize = 10_000;
const SUMMARY_ITEMS: usize = 20;
/// Room for the truncation notice, which names a spill path.
const MIN_MAX_BYTES: usize = 1024;
const MIN_MAX_LINES: usize = 8;
/// Room for the smallest details fallback, which names a spill path.
const MIN_DETAILS_MAX_BYTES: usize = 1024;
const KEY_MAX_BYTES: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub max_bytes: usize,
    pub max_lines: usize,
    pub details_max_bytes: usize,
}

/// `settings.outputGuard` from `mcp.json`; `None` disables text and details guarding.
pub(crate) fn resolve(setting: Option<&Value>) -> Option<Limits> {
    resolve_with(
        std::env::var("ICERS_MCP_OUTPUT_GUARD").ok().as_deref(),
        setting,
    )
}

fn resolve_with(env: Option<&str>, setting: Option<&Value>) -> Option<Limits> {
    let enabled = match env.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("0" | "false" | "no" | "off") => false,
        Some("1" | "true" | "yes" | "on") => true,
        _ => setting != Some(&Value::Bool(false)),
    };
    if !enabled {
        return None;
    }
    // Non-positive or non-numeric tuning falls back to the default, as in PMA.
    // Smaller values are raised to a floor that still fits the truncation notice
    // or the details summary, so the result never exceeds the configured limit.
    let tuning = |key: &str, default: usize, floor: usize| {
        setting
            .and_then(|s| s.get(key))
            .and_then(Value::as_f64)
            .map(f64::floor)
            .filter(|n| *n >= 1.0)
            .map_or(default, |n| (n as usize).max(floor))
    };
    Some(Limits {
        max_bytes: tuning("maxBytes", DEFAULT_MAX_BYTES, MIN_MAX_BYTES),
        max_lines: tuning("maxLines", DEFAULT_MAX_LINES, MIN_MAX_LINES),
        details_max_bytes: tuning(
            "detailsMaxBytes",
            DEFAULT_DETAILS_MAX_BYTES,
            MIN_DETAILS_MAX_BYTES,
        ),
    })
}

/// Session-owned spill directory, removed by `cleanup` or when the last owner drops.
#[derive(Default)]
pub(crate) struct SpillStore(Mutex<Spill>);
#[derive(Default)]
struct Spill {
    dir: Option<tempfile::TempDir>,
    closed: bool,
    sequence: u64,
    blob_bytes: usize,
    blob_files: usize,
}
impl SpillStore {
    /// Writes a new mode-600 file; `blob` counts toward the session blob caps.
    fn write(&self, name: &str, bytes: &[u8], blob: bool) -> Result<PathBuf, String> {
        // Held through the write, so `cleanup` cannot delete the directory under a
        // write in progress, and the quota is charged only for a file that exists.
        let mut spill = self.0.lock().unwrap();
        if spill.closed {
            return Err("session ended".into());
        }
        if blob {
            if bytes.len() > MAX_BLOB_BYTES {
                return Err("decoded size exceeds 10 MiB".into());
            }
            if spill.blob_files >= MAX_SESSION_BLOB_FILES
                || spill.blob_bytes + bytes.len() > MAX_SESSION_BLOB_BYTES
            {
                return Err("session resource limit reached".into());
            }
        }
        if spill.dir.is_none() {
            let mut builder = tempfile::Builder::new();
            builder.prefix("icers-mcp-");
            // Otherwise the directory gets the umask default, typically 755.
            #[cfg(unix)]
            builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
            spill.dir = Some(builder.tempdir().map_err(|e| e.to_string())?);
        }
        spill.sequence += 1;
        let path = spill
            .dir
            .as_ref()
            .unwrap()
            .path()
            .join(format!("{name}-{}", spill.sequence));
        let written = create_private(&path).and_then(|mut file| file.write_all(bytes));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&path);
            return Err(error.to_string());
        }
        if blob {
            spill.blob_bytes += bytes.len();
            spill.blob_files += 1;
        }
        Ok(path)
    }

    /// Session teardown: deletes every spill file; later writes fail.
    pub fn cleanup(&self) {
        let mut spill = self.0.lock().unwrap();
        spill.closed = true;
        if let Some(dir) = spill.dir.take() {
            let _ = dir.close();
        }
    }
}

fn create_private(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Images go to the model as base64, so they get the blob size cap and must decode.
/// Active even when the guard is off, like blob materialization.
pub(crate) fn check_image(data: &str) -> Result<(), String> {
    if data.len() / 4 * 3 > MAX_BLOB_BYTES + 3 {
        return Err("decoded size exceeds 10 MiB".into());
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map(drop)
        .map_err(|_| "invalid base64".into())
}

#[derive(Clone, Default)]
pub(crate) struct OutputGuard {
    pub store: Arc<SpillStore>,
    pub limits: Option<Limits>,
}
impl OutputGuard {
    /// Replaces an embedded `blob` with a file reference, also in the raw result,
    /// so base64 never reaches the model or `details`. Active even when the guard is off.
    pub fn materialize_blob(&self, resource: &mut Value) -> String {
        let uri = resource["uri"].as_str().unwrap_or("(no URI)").to_owned();
        let mime = resource["mimeType"]
            .as_str()
            .unwrap_or("application/octet-stream")
            .to_owned();
        let blob = resource["blob"].as_str().unwrap_or_default();
        // Reject before decoding so an oversized blob is never allocated twice.
        let estimate = blob.len() / 4 * 3;
        let saved = if estimate > MAX_BLOB_BYTES + 3 {
            Err("decoded size exceeds 10 MiB".to_string())
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(blob)
                .map_err(|_| "invalid base64".to_string())
                .and_then(|bytes| self.store.write("resource", &bytes, true))
        };
        let status = match saved {
            Ok(path) => format!("Binary content saved to {}", path.display()),
            Err(reason) => format!("Binary content omitted: {reason}"),
        };
        let text = format!("[Resource: {uri}]\n{status}\nMIME type: {mime}");
        if let Some(object) = resource.as_object_mut() {
            object.remove("blob");
            object.insert("text".into(), json!(text));
        }
        text
    }

    /// Text over the limits becomes a head preview plus a notice naming the spill file,
    /// in place of the first text block. Images keep their positions.
    pub fn bound_content(&self, content: Vec<TextContentOrImage>) -> Vec<TextContentOrImage> {
        let Some(limits) = self.limits else {
            return content;
        };
        let text = content
            .iter()
            .filter_map(|block| match block {
                TextContentOrImage::Text(text) => Some(text.text.as_str()),
                TextContentOrImage::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !over(&text, limits) {
            return content;
        }
        let mut truncated = Some(self.truncate(&text, limits));
        content
            .into_iter()
            .filter_map(|block| match block {
                TextContentOrImage::Text(_) => truncated.take().map(TextContentOrImage::text),
                image => Some(image),
            })
            .collect()
    }

    pub fn bound_text(&self, text: String) -> String {
        match self.limits {
            Some(limits) if over(&text, limits) => self.truncate(&text, limits),
            _ => text,
        }
    }

    fn truncate(&self, text: &str, limits: Limits) -> String {
        let lines = split_lines_for_counting(text).len();
        let notice = match self.store.write("output", text.as_bytes(), false) {
            Ok(path) => format!(
                "[MCP text output truncated: original {lines} lines / {}. Full text saved to: {} — use read with offset/limit or grep to inspect.]",
                format_size(text.len()),
                path.display()
            ),
            Err(error) => format!(
                "[MCP text output truncated: original {lines} lines / {}. Full output could not be saved: {error}]",
                format_size(text.len())
            ),
        };
        let reserved = format!("\n\n{notice}");
        let max_bytes = limits.max_bytes.saturating_sub(reserved.len());
        let max_lines = limits
            .max_lines
            .saturating_sub(split_lines_for_counting(&reserved).len());
        let head = truncate_head(
            text,
            TruncationOptions {
                max_lines: Some(max_lines),
                max_bytes: Some(max_bytes),
            },
        );
        // MCP servers often return one huge JSON line; cut it rather than show nothing.
        let preview = if head.first_line_exceeds_limit && max_lines > 0 {
            let mut end = max_bytes.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            head.content.as_str()
        };
        format!("{preview}{reserved}")
    }

    /// Raw result up to `detailsMaxBytes`, otherwise a summary plus a spill file.
    pub fn bound_details(&self, result: Value) -> Value {
        let Some(limits) = self.limits else {
            return result;
        };
        let raw = result.to_string();
        if raw.len() <= limits.details_max_bytes {
            return result;
        }
        let (path, error) = match self.store.write("mcp-result", raw.as_bytes(), false) {
            Ok(path) => (Some(path.display().to_string()), None),
            Err(error) => (None, Some(error)),
        };
        let content = result["content"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut blocks: Vec<Value> = content
            .iter()
            .take(SUMMARY_ITEMS)
            .map(summarize_block)
            .collect();
        if content.len() > SUMMARY_ITEMS {
            blocks.push(json!({"type":"omitted","count":content.len() - SUMMARY_ITEMS}));
        }
        let mut summary = json!({
            "omitted": true,
            "reason": "Raw MCP result exceeded the details size limit and was replaced with this summary to keep session context bounded.",
            "isError": result["isError"] == true,
            "contentBlocks": content.len(),
            "contentSummary": blocks,
            "rawResultBytes": raw.len(),
        });
        if let Some(keys) = result.as_object() {
            summary["keys"] = keys
                .keys()
                .take(SUMMARY_ITEMS)
                .map(|key| json!(bounded_key(key)))
                .collect();
        }
        if let Some(path) = &path {
            summary["fullResultPath"] = json!(path);
        }
        if let Some(error) = error {
            summary["resultWriteError"] = json!(error);
        }
        let fallbacks = [
            json!({"omitted":true,"rawResultBytes":raw.len(),"fullResultPath":path}),
            json!({"omitted":true,"fullResultPath":path}),
            json!({"omitted":true,"rawResultBytes":raw.len()}),
        ];
        std::iter::once(summary)
            .chain(fallbacks)
            .find(|candidate| candidate.to_string().len() <= limits.details_max_bytes)
            .unwrap_or_else(|| json!({"omitted":true}))
    }
}

fn over(text: &str, limits: Limits) -> bool {
    text.len() > limits.max_bytes || split_lines_for_counting(text).len() > limits.max_lines
}

fn summarize_block(block: &Value) -> Value {
    match block["type"].as_str() {
        Some("text") => {
            let text = block["text"].as_str().unwrap_or_default();
            json!({"type":"text","bytes":text.len(),"lines":split_lines_for_counting(text).len(),"textOmitted":true})
        }
        Some("image") => {
            json!({"type":"image","mimeType":block["mimeType"],"dataBytes":block["data"].as_str().map_or(0, str::len),"dataOmitted":true})
        }
        kind => {
            json!({"type":kind.map(bounded_key).unwrap_or_else(|| "unknown".into()),"estimatedBytes":block.to_string().len(),"omitted":true})
        }
    }
}

fn bounded_key(key: &str) -> String {
    if key.len() <= KEY_MAX_BYTES {
        return key.to_owned();
    }
    let mut end = KEY_MAX_BYTES - '…'.len_utf8();
    while !key.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &key[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limited(max_bytes: usize, max_lines: usize, details_max_bytes: usize) -> OutputGuard {
        OutputGuard {
            store: Arc::default(),
            limits: Some(Limits {
                max_bytes,
                max_lines,
                details_max_bytes,
            }),
        }
    }

    fn text(block: &TextContentOrImage) -> &str {
        match block {
            TextContentOrImage::Text(text) => &text.text,
            TextContentOrImage::Image(_) => panic!("expected text"),
        }
    }

    fn saved_path(text: &str, marker: &str) -> PathBuf {
        let start = text.find(marker).expect(text) + marker.len();
        let end = text[start..].find(" — ").unwrap_or(text.len() - start);
        PathBuf::from(&text[start..start + end])
    }

    #[test]
    fn settings_and_env_kill_switch() {
        let defaults = Limits {
            max_bytes: DEFAULT_MAX_BYTES,
            max_lines: DEFAULT_MAX_LINES,
            details_max_bytes: DEFAULT_DETAILS_MAX_BYTES,
        };
        assert_eq!(resolve_with(None, None), Some(defaults));
        assert_eq!(resolve_with(None, Some(&json!(true))), Some(defaults));
        assert_eq!(resolve_with(None, Some(&json!(false))), None);
        let tuned = json!({"maxBytes":4000.9,"maxLines":0,"detailsMaxBytes":"x"});
        assert_eq!(
            resolve_with(None, Some(&tuned)),
            Some(Limits {
                max_bytes: 4000,
                ..defaults
            })
        );
        // Values too small for the notice are raised to the floor.
        let tiny = json!({"maxBytes":1,"maxLines":1,"detailsMaxBytes":1});
        assert_eq!(
            resolve_with(None, Some(&tiny)),
            Some(Limits {
                max_bytes: MIN_MAX_BYTES,
                max_lines: MIN_MAX_LINES,
                details_max_bytes: MIN_DETAILS_MAX_BYTES,
            })
        );
        for off in ["0", "false", " OFF ", "no"] {
            assert_eq!(resolve_with(Some(off), None), None, "{off}");
        }
        assert_eq!(resolve_with(Some("1"), Some(&json!(false))), Some(defaults));
        assert_eq!(resolve_with(Some("maybe"), Some(&json!(false))), None);
    }

    #[test]
    fn truncation_keeps_images_in_place_and_images_are_capped() {
        let guard = limited(200, 10, 1024);
        let image = |data: &str| {
            TextContentOrImage::Image(ice_ai::types::ImageContent {
                kind: Default::default(),
                data: data.into(),
                mime_type: "image/png".into(),
            })
        };
        let long = (0..50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let bounded = guard.bound_content(vec![
            TextContentOrImage::text(long),
            image("YWJj"),
            TextContentOrImage::text("caption"),
            image("ZGVm"),
        ]);
        assert!(matches!(&bounded[0], TextContentOrImage::Text(t) if t.text.contains("truncated")));
        assert!(matches!(&bounded[1], TextContentOrImage::Image(i) if i.data == "YWJj"));
        assert!(matches!(&bounded[2], TextContentOrImage::Image(i) if i.data == "ZGVm"));
        assert_eq!(bounded.len(), 3);

        assert_eq!(check_image("YWJj"), Ok(()));
        assert_eq!(check_image("not base64!"), Err("invalid base64".into()));
        let huge = "A".repeat((MAX_BLOB_BYTES + 8) / 3 * 4);
        assert_eq!(
            check_image(&huge),
            Err("decoded size exceeds 10 MiB".into())
        );
    }

    #[test]
    fn oversize_text_is_truncated_with_private_spill_and_images_pass_through() {
        let guard = limited(200, 10, 1024);
        let full = (0..50).map(|i| format!("line {i}")).collect::<Vec<_>>();
        let image = TextContentOrImage::Image(ice_ai::types::ImageContent {
            kind: Default::default(),
            data: "YWJj".into(),
            mime_type: "image/png".into(),
        });
        let content = vec![
            TextContentOrImage::text(full[..25].join("\n")),
            image.clone(),
            TextContentOrImage::text(full[25..].join("\n")),
        ];
        let bounded = guard.bound_content(content);
        assert_eq!(bounded.len(), 2);
        assert_eq!(bounded[1], image);
        let shown = text(&bounded[0]);
        assert!(shown.starts_with("line 0\nline 1\n"), "{shown}");
        assert!(shown.contains("original 50 lines"), "{shown}");
        assert!(shown.len() <= 200 && split_lines_for_counting(shown).len() <= 10);
        let path = saved_path(shown, "Full text saved to: ");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), full.join("\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode();
            assert_eq!(mode(&path) & 0o777, 0o600);
            assert_eq!(mode(path.parent().unwrap()) & 0o777, 0o700);
        }
        // Small output is untouched.
        let small = vec![TextContentOrImage::text("ok")];
        assert_eq!(guard.bound_content(small.clone()), small);
        // One huge line still yields a preview instead of an empty head.
        let line = guard.bound_text("é".repeat(500));
        assert!(line.starts_with("éé") && line.len() <= 200, "{line}");
        drop(guard);
        assert!(
            !path.exists(),
            "spill files are removed when the session drops"
        );
    }

    #[test]
    fn disabled_guard_keeps_text_and_details_raw() {
        let guard = OutputGuard::default();
        let big = "x".repeat(DEFAULT_MAX_BYTES * 2);
        assert_eq!(guard.bound_text(big.clone()), big);
        let details = json!({"content":[{"type":"text","text":big}]});
        assert_eq!(guard.bound_details(details.clone()), details);
    }

    #[test]
    fn large_details_become_a_summary_with_a_spill_file() {
        let guard = limited(DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, 2048);
        let small = json!({"content":[{"type":"text","text":"ok"}]});
        assert_eq!(guard.bound_details(small.clone()), small);
        let mut content: Vec<Value> = (0..25)
            .map(|_| json!({"type":"text","text":"x".repeat(100)}))
            .collect();
        content[0] = json!({"type":"image","mimeType":"image/png","data":"YWJj"});
        let raw = json!({"content":content,"isError":false,"structuredContent":{"a":1}});
        let summary = guard.bound_details(raw.clone());
        assert_eq!(summary["omitted"], true);
        assert_eq!(summary["contentBlocks"], 25);
        assert_eq!(summary["contentSummary"][0]["dataBytes"], 4);
        assert_eq!(summary["contentSummary"][1]["bytes"], 100);
        assert_eq!(summary["contentSummary"][20]["count"], 5);
        assert_eq!(
            summary["keys"],
            json!(["content", "isError", "structuredContent"])
        );
        assert!(summary.to_string().len() <= 2048);
        let path = summary["fullResultPath"].as_str().unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(saved, raw);
        // A limit below the summary size falls back to the smallest marker that fits.
        let tight = limited(DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, 40);
        assert_eq!(
            tight.bound_details(raw),
            json!({"omitted":true,"rawResultBytes":raw_len(&summary)})
        );
    }

    fn raw_len(summary: &Value) -> u64 {
        summary["rawResultBytes"].as_u64().unwrap()
    }

    #[test]
    fn blobs_are_materialized_within_session_caps_even_when_disabled() {
        let guard = OutputGuard::default();
        let mut resource =
            json!({"uri":"file:///a.bin","mimeType":"application/x","blob":"aGVsbG8="});
        let shown = guard.materialize_blob(&mut resource);
        assert!(shown.starts_with("[Resource: file:///a.bin]\nBinary content saved to "));
        assert!(shown.ends_with("\nMIME type: application/x"));
        let path = saved_path(shown.lines().nth(1).unwrap(), "saved to ");
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert!(resource.get("blob").is_none());
        assert_eq!(resource["text"], shown);

        let oversize =
            base64::engine::general_purpose::STANDARD.encode(vec![0; MAX_BLOB_BYTES + 1]);
        let mut resource = json!({ "blob": oversize });
        assert!(guard
            .materialize_blob(&mut resource)
            .contains("omitted: decoded size exceeds 10 MiB"));
        let mut resource = json!({"blob":"not base64!"});
        assert!(guard
            .materialize_blob(&mut resource)
            .contains("omitted: invalid base64"));

        guard.store.0.lock().unwrap().blob_bytes = MAX_SESSION_BLOB_BYTES - 4;
        let mut resource = json!({"blob":"aGVsbG8="});
        assert!(guard
            .materialize_blob(&mut resource)
            .contains("omitted: session resource limit reached"));
        let mut spill = guard.store.0.lock().unwrap();
        spill.blob_bytes = 0;
        spill.blob_files = MAX_SESSION_BLOB_FILES;
        drop(spill);
        let mut resource = json!({"blob":"aGVsbG8="});
        assert!(guard
            .materialize_blob(&mut resource)
            .contains("session resource limit reached"));

        guard.store.cleanup();
        assert!(!path.exists(), "teardown removes materialized blobs");
        guard.store.0.lock().unwrap().blob_files = 0;
        let mut resource = json!({"blob":"aGVsbG8="});
        assert!(guard
            .materialize_blob(&mut resource)
            .contains("omitted: session ended"));
    }
}
