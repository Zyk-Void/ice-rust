//! Mirrors `packages/agent/src/harness/tools/read.ts` — the built-in `read`
//! `AgentTool`. Reads a file (text or image), applies head-truncation with a
//! continuation hint, and detects supported image MIME types. An optional
//! `ReadImageProcessor` seam supports resize/convert implementations; without
//! one, BMP images yield an omitted notice and other formats are inlined.

use std::sync::Arc;

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_ai::types::{
    ConstrainedSamplingConfig, ConstrainedStrictness, ImageContent, ImageContentType, Tool,
};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::env::ExecutionEnv;
use crate::image::{detect_supported_image_mime_type, encode_base64};
use crate::path_utils::resolve_read_tool_path;
use crate::tools::tool_context::ExecutionToolContext;
use crate::truncate::{
    format_size, truncate_head, TruncationOptions, TruncationResult, DEFAULT_MAX_BYTES,
    DEFAULT_MAX_LINES,
};
/// Input for the read tool. Mirrors TS `ReadToolInput`. `offset` is 1-indexed.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct ReadInput {
    /// Path to the file to read (relative or absolute).
    pub path: String,
    /// Line number to start reading from (1-indexed).
    #[serde(default)]
    pub offset: Option<f64>,
    /// Maximum number of lines to read.
    #[serde(default)]
    pub limit: Option<f64>,
}

/// Text ice shows when the current model cannot take image input.
const NON_VISION_WARNING: &str =
    "[Current model does not support images. The image will be omitted from this request.]";

/// Coerce a JSON number to a whole line count. ice accepts any number here;
/// icers floors fractional values (some providers send `10.0`) and rejects
/// non-finite ones.
fn line_number(value: Option<f64>, name: &str) -> Result<Option<usize>, AgentError> {
    match value {
        None => Ok(None),
        Some(v) if !v.is_finite() => {
            Err(AgentError::Tool(format!("{name} must be a finite number")))
        }
        Some(v) => Ok(Some(v.max(0.0).floor() as usize)),
    }
}

/// Structured details for the read tool. Mirrors TS `ReadToolDetails`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReadToolDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationResult>,
}

/// Options for `create_read_tool`. Mirrors TS `ReadToolOptions`.
pub struct ReadToolOptions {
    /// Whether an injected image processor should resize images. Default true.
    pub auto_resize_images: bool,
    /// Optional image conversion/resizing implementation.
    pub image_processor: Option<Arc<dyn ReadImageProcessor>>,
}

impl Clone for ReadToolOptions {
    fn clone(&self) -> Self {
        Self {
            auto_resize_images: self.auto_resize_images,
            image_processor: self.image_processor.clone(),
        }
    }
}

impl std::fmt::Debug for ReadToolOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadToolOptions")
            .field("auto_resize_images", &self.auto_resize_images)
            .field("image_processor", &self.image_processor.is_some())
            .finish()
    }
}

impl Default for ReadToolOptions {
    fn default() -> Self {
        Self {
            auto_resize_images: true,
            image_processor: None,
        }
    }
}

/// Optional image-conversion seam. Mirrors TS `ReadImageProcessor`.
pub trait ReadImageProcessor: Send + Sync {
    fn process(
        &self,
        bytes: &[u8],
        mime_type: &str,
        auto_resize: bool,
    ) -> Result<ProcessedImage, String>;
}

/// Result of a successful [`ReadImageProcessor::process`].
pub struct ProcessedImage {
    pub data: String,
    pub mime_type: String,
    pub hints: Vec<String>,
}

/// The built-in `read` tool. Holds an `Arc<dyn ExecutionEnv>` (read-only view)
/// and an optional image processor.
pub struct ReadTool {
    schema: Tool,
    env: Arc<dyn ExecutionEnv>,
    options: ReadToolOptions,
    image_processor: Option<Arc<dyn ReadImageProcessor>>,
}

impl ReadTool {
    fn schema() -> Tool {
        let params = schemars::schema_for!(ReadInput);
        Tool {
            name: "read".to_string(),
            description: format!(
                "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). \
                 Images are sent as attachments. For text files, output is truncated to {DEFAULT_MAX_LINES} \
                 lines or {kb}KB (whichever is hit first). Use offset/limit for large files. When you need \
                 the full file, continue with offset until complete.",
                kb = DEFAULT_MAX_BYTES / 1024
            ),
            parameters: ice_ai::types::Schema::new(serde_json::to_value(params).unwrap_or_default()),
            constrained_sampling: Some(ConstrainedSamplingConfig::JsonSchema { strict: ConstrainedStrictness::Prefer }),
        }
    }
}

/// Build the `read` tool from a context. Mirrors TS `createReadTool`.
pub fn create_read_tool(
    context: &ExecutionToolContext,
    options: Option<ReadToolOptions>,
) -> Arc<dyn AgentTool> {
    let options = options.unwrap_or_default();
    let image_processor = options.image_processor.clone();
    Arc::new(ReadTool {
        schema: ReadTool::schema(),
        env: context.env().clone(),
        options,
        image_processor,
    })
}

#[async_trait]
impl AgentTool for ReadTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "read"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: ReadInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("read input invalid: {e}")))?;
        let offset = line_number(input.offset, "offset")?;
        let limit = line_number(input.limit, "limit")?;

        let abs = resolve_read_tool_path(&*self.env, &input.path, Some(&signal))
            .await
            .map_err(file_err_to_agent)?;
        let bytes = self
            .env
            .read_binary_file(&abs, Some(&signal))
            .await
            .map_err(file_err_to_agent)?;

        // Image path.
        if let Some(mime) = detect_supported_image_mime_type(&bytes) {
            let mut result = image_result(
                mime.to_string(),
                bytes,
                self.options.auto_resize_images,
                self.image_processor.clone(),
                &signal,
            )
            .await?;
            let non_vision = ice_agent::tool_execution_scope::tool_model()
                .is_some_and(|m| !m.input.contains(&ice_ai::InputModality::Image));
            let has_image = result
                .content
                .iter()
                .any(|c| matches!(c, TextContentOrImage::Image(_)));
            if non_vision && has_image {
                if let Some(TextContentOrImage::Text(t)) = result.content.first_mut() {
                    t.text = format!("{NON_VISION_WARNING}\n{}", t.text);
                }
            }
            return Ok(result);
        }

        // Text path. A leading UTF-8 BOM is dropped, as the harness
        // TextDecoder does.
        let text_content = String::from_utf8_lossy(&bytes).into_owned();
        let text_content = text_content
            .strip_prefix('\u{FEFF}')
            .map(str::to_owned)
            .unwrap_or(text_content);
        let all_lines: Vec<&str> = text_content.split('\n').collect();
        // Mirror TS: `textContent.split("\n")` keeps a trailing "" when the
        // content ends with '\n'; the line-count + slice math below uses the raw
        // split (NOT `split_lines_for_counting`), so we keep it as-is.
        let total_file_lines = all_lines.len();
        let start_line = offset.map(|o| o.saturating_sub(1)).unwrap_or(0);
        let start_line_display = start_line + 1;
        if start_line >= all_lines.len() {
            return Err(AgentError::Tool(format!(
                "Offset {} is beyond end of file ({} lines total)",
                offset.unwrap_or(1),
                all_lines.len()
            )));
        }

        let (selected_content, user_limited_lines): (String, Option<usize>) =
            if let Some(limit) = limit {
                let end_line = std::cmp::min(start_line + limit, all_lines.len());
                (
                    all_lines[start_line..end_line].join("\n"),
                    Some(end_line - start_line),
                )
            } else {
                (all_lines[start_line..].join("\n"), None)
            };

        let truncation = truncate_head(&selected_content, TruncationOptions::default());
        let (output_text, details): (String, Option<TruncationResult>) = if truncation
            .first_line_exceeds_limit
        {
            let first_line_size = format_size(all_lines[start_line].as_bytes().len());
            (
                    format!(
                        "[Line {start_line_display} is {first_line_size}, exceeds {} limit. \
                         Use bash: sed -n '{start_line_display}p' {} | head -c {DEFAULT_MAX_BYTES}]",
                        format_size(DEFAULT_MAX_BYTES),
                        input.path
                    ),
                    Some(truncation),
                )
        } else if truncation.truncated {
            let end_line_display = start_line_display + truncation.output_lines - 1;
            let next_offset = end_line_display + 1;
            let mut text = truncation.content.clone();
            match truncation.truncated_by {
                Some(crate::truncate::TruncationLimit::Lines) => {
                    text.push_str(&format!(
                        "\n\n[Showing lines {start_line_display}-{end_line_display} of \
                             {total_file_lines}. Use offset={next_offset} to continue.]"
                    ));
                }
                _ => {
                    text.push_str(&format!(
                        "\n\n[Showing lines {start_line_display}-{end_line_display} of \
                             {total_file_lines} ({} limit). Use offset={next_offset} to continue.]",
                        format_size(DEFAULT_MAX_BYTES)
                    ));
                }
            }
            (text, Some(truncation))
        } else if let Some(ull) = user_limited_lines {
            if start_line + ull < all_lines.len() {
                let remaining = all_lines.len() - (start_line + ull);
                let next_offset = start_line + ull + 1;
                (
                        format!(
                            "{}\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]",
                            truncation.content
                        ),
                        None,
                    )
            } else {
                (truncation.content, None)
            }
        } else {
            (truncation.content, None)
        };

        let details_value = match details {
            Some(t) => serde_json::to_value(ReadToolDetails {
                truncation: Some(t),
            })
            .unwrap_or(serde_json::Value::Null),
            None => serde_json::Value::Null,
        };
        Ok(AgentToolResult {
            content: vec![TextContentOrImage::text(output_text)],
            details: details_value,
            ..Default::default()
        })
    }
}

/// Build the image-path result, honoring an optional processor. Mirrors the TS
/// image branch. Decoding and resizing are CPU-bound, so the processor runs on
/// the blocking pool; a cancelled call returns immediately and the late result
/// is discarded.
async fn image_result(
    mime: String,
    bytes: Vec<u8>,
    auto_resize: bool,
    processor: Option<Arc<dyn ReadImageProcessor>>,
    signal: &CancellationToken,
) -> Result<AgentToolResult, AgentError> {
    let aborted = || AgentError::Tool("Operation aborted".into());
    if signal.is_cancelled() {
        return Err(aborted());
    }
    if let Some(proc) = processor {
        let job_mime = mime.clone();
        let job = tokio::task::spawn_blocking(move || proc.process(&bytes, &job_mime, auto_resize));
        let processed = tokio::select! {
            _ = signal.cancelled() => return Err(aborted()),
            joined = job => joined.unwrap_or_else(|e| Err(format!("[Image omitted: processing failed ({e}).]"))),
        };
        return Ok(processed_image_result(&mime, processed));
    }
    Ok(unprocessed_image_result(&mime, &bytes))
}

fn processed_image_result(
    mime: &str,
    processed: Result<ProcessedImage, String>,
) -> AgentToolResult {
    {
        match processed {
            Ok(p) => {
                let hints = if p.hints.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", p.hints.join("\n"))
                };
                return AgentToolResult {
                    content: vec![
                        TextContentOrImage::text(format!(
                            "Read image file [{}]{}",
                            p.mime_type, hints
                        )),
                        TextContentOrImage::Image(ImageContent {
                            kind: ImageContentType,
                            data: p.data,
                            mime_type: p.mime_type,
                        }),
                    ],
                    details: serde_json::Value::Null,
                    ..Default::default()
                };
            }
            Err(message) => {
                return AgentToolResult {
                    content: vec![TextContentOrImage::text(format!(
                        "Read image file [{mime}]\n{message}"
                    ))],
                    details: serde_json::Value::Null,
                    ..Default::default()
                };
            }
        }
    }
}

fn unprocessed_image_result(mime: &str, bytes: &[u8]) -> AgentToolResult {
    // No processor — BMP is omitted; others are inlined.
    if mime == "image/bmp" {
        return AgentToolResult {
            content: vec![TextContentOrImage::text(
                "Read image file [image/bmp]\n[Image omitted: configure an imageProcessor to convert BMP images.]",
            )],
            details: serde_json::Value::Null,
            ..Default::default()
        };
    }
    AgentToolResult {
        content: vec![
            TextContentOrImage::text(format!("Read image file [{mime}]")),
            TextContentOrImage::Image(ImageContent {
                kind: ImageContentType,
                data: encode_base64(bytes),
                mime_type: mime.to_string(),
            }),
        ],
        details: serde_json::Value::Null,
        ..Default::default()
    }
}

/// Map a `FileError` into an `AgentError::Tool`, mirroring the TS `getOrThrow`
/// surface (the message carries the path + code).
fn file_err_to_agent(e: crate::error::FileError) -> AgentError {
    AgentError::Tool(e.to_string())
}
