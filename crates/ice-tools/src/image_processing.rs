//! Image processing utilities — resize, convert, and transform images.
//!
//! Mirrors `packages/coding-agent/src/utils/image-*.ts`. Provides cross-platform
//! image processing for the agent's vision capabilities.
//!
//! # Features
//!
//! - **Resize**: Scale images while maintaining aspect ratio
//! - **Convert**: Convert between image formats (PNG, JPEG, WebP)
//! - **EXIF handling**: Strip EXIF data for privacy
//! - **Optimization**: Reduce file size for API transmission
//!
//! # Example
//!
//! ```rust,no_run
//! use ice_tools::image_processing::{process_image, ImageProcessingOptions, OutputFormat};
//!
//! # fn example(input_bytes: &[u8]) -> Result<(), String> {
//! let options = ImageProcessingOptions {
//!     max_width: Some(1024),
//!     max_height: Some(1024),
//!     format: Some(OutputFormat::Jpeg { quality: 85 }),
//!     strip_exif: true,
//! };
//!
//! let processed = process_image(input_bytes, options)?;
//! # Ok(())
//! # }
//! ```

use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

/// Pi's default inline limits: 2000 pixels per side and 4.5 MiB of base64.
pub const INLINE_IMAGE_MAX_SIDE: u32 = 2000;
pub const INLINE_IMAGE_MAX_BASE64_BYTES: usize = 9 * 1024 * 1024 / 2;

/// Default JPEG quality tried first. Mirrors `ImageResizeOptions.jpegQuality`.
pub const INLINE_IMAGE_JPEG_QUALITY: u8 = 80;

/// Decode, orient, resize and encode an image for model input. Mirrors
/// `resizeImage` + `processImage`:
///
/// - A supported image already within 2000x2000 and the base64 budget keeps
///   its original bytes (only when no EXIF orientation transform applies;
///   ice keeps the original bytes even then, which shows the wrong rotation).
/// - Otherwise fit the oriented image within 2000x2000 and, at each candidate
///   size, resize from the original decoded image and try PNG, then JPEG at
///   qualities 80, 85, 70, 55 and 40, returning the first encoding strictly
///   under the budget. If none fits, shrink both sides by 0.75 and repeat
///   down to 1x1.
/// - A resized or re-encoded result carries ice's coordinate hint.
pub fn prepare_inline_image(bytes: &[u8]) -> Result<crate::tools::read::ProcessedImage, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use image::ImageDecoder;
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let original = (img.width(), img.height());
    let mime = crate::image::detect_supported_image_mime_type(bytes);
    let fits = |len: usize| len.div_ceil(3) * 4 < INLINE_IMAGE_MAX_BASE64_BYTES;
    if matches!(
        mime,
        Some("image/png" | "image/jpeg" | "image/gif" | "image/webp")
    ) && original.0 <= INLINE_IMAGE_MAX_SIDE
        && original.1 <= INLINE_IMAGE_MAX_SIDE
        && fits(bytes.len())
        && orientation == image::metadata::Orientation::NoTransforms
    {
        return Ok(crate::tools::read::ProcessedImage {
            data: STANDARD.encode(bytes),
            mime_type: mime.unwrap().into(),
            hints: vec![],
        });
    }

    let (mut width, mut height) =
        fit_within(original, INLINE_IMAGE_MAX_SIDE, INLINE_IMAGE_MAX_SIDE);
    let mut qualities = vec![INLINE_IMAGE_JPEG_QUALITY];
    for q in [85, 70, 55, 40] {
        if !qualities.contains(&q) {
            qualities.push(q);
        }
    }
    loop {
        let candidate = if (width, height) == original {
            img.clone()
        } else {
            img.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
        };
        if let Some((data, encoded_mime)) = encode_within_budget(&candidate, &qualities, &fits)? {
            let mut hints = vec![];
            if let Some(mime) = mime {
                if mime != encoded_mime {
                    hints.push(format!("[Image converted from {mime} to {encoded_mime}.]"));
                }
            }
            hints.push(format_dimension_note(original, (width, height)));
            return Ok(crate::tools::read::ProcessedImage {
                data: STANDARD.encode(data),
                mime_type: encoded_mime.into(),
                hints,
            });
        }
        if (width, height) == (1, 1) {
            return Err("Image could not be resized below the inline image size limit".into());
        }
        width = ((width as f64 * 0.75).floor() as u32).max(1);
        height = ((height as f64 * 0.75).floor() as u32).max(1);
    }
}

/// Constrain width, then height, rounding like ice's `Math.round`.
fn fit_within((w, h): (u32, u32), max_w: u32, max_h: u32) -> (u32, u32) {
    let (mut w, mut h) = (w as f64, h as f64);
    if w > max_w as f64 {
        h = (h * max_w as f64 / w).round();
        w = max_w as f64;
    }
    if h > max_h as f64 {
        w = (w * max_h as f64 / h).round();
        h = max_h as f64;
    }
    ((w as u32).max(1), (h as u32).max(1))
}

/// The first encoding strictly under the budget: PNG, then each JPEG quality.
fn encode_within_budget(
    img: &DynamicImage,
    qualities: &[u8],
    fits: &dyn Fn(usize) -> bool,
) -> Result<Option<(Vec<u8>, &'static str)>, String> {
    let png = encode_image(img, Some(OutputFormat::Png))?;
    if fits(png.len()) {
        return Ok(Some((png, "image/png")));
    }
    // JPEG cannot encode alpha; composite onto white first.
    let mut rgb = img.to_rgb8();
    let rgba = img.to_rgba8();
    for (target, source) in rgb.pixels_mut().zip(rgba.pixels()) {
        let alpha = source[3] as u16;
        for channel in 0..3 {
            target[channel] = ((source[channel] as u16 * alpha + 255 * (255 - alpha)) / 255) as u8;
        }
    }
    let rgb = DynamicImage::ImageRgb8(rgb);
    for &quality in qualities {
        let jpeg = encode_image(&rgb, Some(OutputFormat::Jpeg { quality }))?;
        if fits(jpeg.len()) {
            return Ok(Some((jpeg, "image/jpeg")));
        }
    }
    Ok(None)
}

/// Mirrors `formatDimensionNote`: how displayed coordinates map back to the
/// original image.
pub fn format_dimension_note(original: (u32, u32), displayed: (u32, u32)) -> String {
    format!(
        "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {:.2} to map to original image.]",
        original.0,
        original.1,
        displayed.0,
        displayed.1,
        original.0 as f64 / displayed.0 as f64
    )
}

/// Shared processor for the built-in read tool.
pub struct InlineImageProcessor;

/// Normalize image blocks while retaining order and surfacing omissions as
/// text, rather than sending an invalid image to a provider.
pub fn normalize_image_blocks(blocks: &mut Vec<ice_ai::types::Content>) {
    use ice_ai::types::Content;
    let mut output = Vec::with_capacity(blocks.len());
    for block in std::mem::take(blocks) {
        if let Content::Image(mut img) = block {
            use base64::{engine::general_purpose::STANDARD, Engine};
            match STANDARD
                .decode(&img.data)
                .map_err(|e| e.to_string())
                .and_then(|bytes| prepare_inline_image(&bytes))
            {
                Ok(processed) => {
                    img.data = processed.data;
                    img.mime_type = processed.mime_type;
                    output.push(Content::Image(img));
                    output.extend(processed.hints.into_iter().map(Content::text));
                }
                Err(error) => output.push(Content::text(format!("[Image omitted: {error}.]"))),
            }
        } else {
            output.push(block);
        }
    }
    *blocks = output;
}

impl crate::tools::read::ReadImageProcessor for InlineImageProcessor {
    fn process(
        &self,
        bytes: &[u8],
        _mime_type: &str,
        auto_resize: bool,
    ) -> Result<crate::tools::read::ProcessedImage, String> {
        if auto_resize {
            return prepare_inline_image(bytes);
        }
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mime = crate::image::detect_supported_image_mime_type(bytes);
        let (bytes, out_mime, hints) = if matches!(
            mime,
            Some("image/png" | "image/jpeg" | "image/gif" | "image/webp")
        ) {
            (bytes.to_vec(), mime.unwrap(), vec![])
        } else {
            let converted = process_image(bytes, ImageProcessingOptions::default())
                .map_err(|e| format!("[Image omitted: could not convert image ({e}).]"))?;
            let from = mime.unwrap_or(_mime_type);
            (
                converted,
                "image/png",
                vec![format!("[Image converted from {from} to image/png.]")],
            )
        };
        Ok(crate::tools::read::ProcessedImage {
            data: STANDARD.encode(bytes),
            mime_type: out_mime.into(),
            hints,
        })
    }
}

/// Output format for processed images.
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    /// PNG format (lossless)
    Png,
    /// JPEG format with quality (1-100)
    Jpeg { quality: u8 },
    /// WebP format with quality (1-100)
    WebP { quality: u8 },
}

/// Options for image processing.
#[derive(Debug, Clone)]
pub struct ImageProcessingOptions {
    /// Maximum width in pixels. If set, image is scaled down to fit.
    pub max_width: Option<u32>,
    /// Maximum height in pixels. If set, image is scaled down to fit.
    pub max_height: Option<u32>,
    /// Output format. If `None`, preserves the input format.
    pub format: Option<OutputFormat>,
    /// Strip EXIF metadata (privacy). Default: `true`.
    pub strip_exif: bool,
}

impl Default for ImageProcessingOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            format: None,
            strip_exif: true,
        }
    }
}

/// Process an image according to the given options.
///
/// # Arguments
///
/// * `input` - Raw image bytes (PNG, JPEG, WebP, etc.)
/// * `options` - Processing options (resize, format, EXIF stripping)
///
/// # Returns
///
/// * `Ok(Vec<u8>)` - Processed image bytes
/// * `Err(String)` - Processing failed
///
/// # Example
///
/// ```rust,no_run
/// use ice_tools::image_processing::{process_image, ImageProcessingOptions, OutputFormat};
///
/// # fn example(raw_bytes: &[u8]) -> Result<(), String> {
/// let options = ImageProcessingOptions {
///     max_width: Some(800),
///     max_height: Some(600),
///     format: Some(OutputFormat::Jpeg { quality: 85 }),
///     strip_exif: true,
/// };
///
/// let processed = process_image(raw_bytes, options)?;
/// # Ok(())
/// # }
/// ```
pub fn process_image(input: &[u8], options: ImageProcessingOptions) -> Result<Vec<u8>, String> {
    // Load the image
    let mut img =
        image::load_from_memory(input).map_err(|e| format!("Failed to load image: {}", e))?;

    // Resize if needed
    if options.max_width.is_some() || options.max_height.is_some() {
        img = resize_image(img, options.max_width, options.max_height);
    }

    // Encode to the desired format
    let output_bytes = encode_image(&img, options.format)?;

    Ok(output_bytes)
}

/// Resize an image to fit within the given bounds while maintaining aspect ratio.
fn resize_image(
    img: DynamicImage,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> DynamicImage {
    let (orig_width, orig_height) = (img.width(), img.height());

    // Calculate the scaling factor
    let width_scale = max_width
        .map(|mw| mw as f64 / orig_width as f64)
        .unwrap_or(1.0);
    let height_scale = max_height
        .map(|mh| mh as f64 / orig_height as f64)
        .unwrap_or(1.0);

    let scale = width_scale.min(height_scale).min(1.0); // Never upscale

    if scale >= 1.0 {
        // No need to resize
        return img;
    }

    let new_width = (orig_width as f64 * scale).round() as u32;
    let new_height = (orig_height as f64 * scale).round() as u32;

    img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
}

/// Encode an image to the specified format.
fn encode_image(img: &DynamicImage, format: Option<OutputFormat>) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);

    match format {
        Some(OutputFormat::Png) => {
            img.write_to(&mut cursor, ImageFormat::Png)
                .map_err(|e| format!("Failed to encode PNG: {}", e))?;
        }
        Some(OutputFormat::Jpeg { quality }) => {
            // For JPEG with quality, we need to use the encoder directly
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
            img.write_with_encoder(encoder)
                .map_err(|e| format!("Failed to encode JPEG: {}", e))?;
        }
        Some(OutputFormat::WebP { quality }) => {
            // WebP encoding requires the webp feature
            #[cfg(feature = "webp")]
            {
                let encoder = image::codecs::webp::WebPEncoder::new_with_quality(
                    &mut cursor,
                    image::codecs::webp::WebPQuality::lossy(quality),
                );
                img.write_with_encoder(encoder)
                    .map_err(|e| format!("Failed to encode WebP: {}", e))?;
            }
            #[cfg(not(feature = "webp"))]
            {
                let _ = quality;
                return Err("WebP encoding requires the 'webp' feature".to_string());
            }
        }
        None => {
            // Default to PNG
            img.write_to(&mut cursor, ImageFormat::Png)
                .map_err(|e| format!("Failed to encode image: {}", e))?;
        }
    }

    Ok(buffer)
}

/// Detect the MIME type of an image from its bytes.
///
/// # Arguments
///
/// * `data` - Raw image bytes
///
/// # Returns
///
/// * `Some(&str)` - MIME type (e.g., "image/png", "image/jpeg")
/// * `None` - Unknown or unsupported format
pub fn detect_image_mime(data: &[u8]) -> Option<&'static str> {
    // Check magic bytes
    if data.len() < 8 {
        return None;
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }

    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }

    // GIF: 47 49 46 38
    if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        return Some("image/gif");
    }

    // WebP: 52 49 46 46 ... 57 45 42 50
    if data.len() >= 12
        && data.starts_with(&[0x52, 0x49, 0x46, 0x46])
        && data[8..12].starts_with(&[0x57, 0x45, 0x42, 0x50])
    {
        return Some("image/webp");
    }

    // BMP: 42 4D
    if data.starts_with(&[0x42, 0x4D]) {
        return Some("image/bmp");
    }

    None
}

/// Get image dimensions without fully decoding the image.
///
/// # Arguments
///
/// * `data` - Raw image bytes
///
/// # Returns
///
/// * `Ok((width, height))` - Image dimensions in pixels
/// * `Err(String)` - Failed to read dimensions
pub fn get_image_dimensions(data: &[u8]) -> Result<(u32, u32), String> {
    let img = image::load_from_memory(data).map_err(|e| format!("Failed to load image: {}", e))?;
    Ok((img.width(), img.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_images_keep_small_bytes_and_resize_large_images() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let small = encode_image(&DynamicImage::new_rgba8(8, 8), Some(OutputFormat::Png)).unwrap();
        let processed = prepare_inline_image(&small).unwrap();
        assert_eq!(STANDARD.decode(processed.data).unwrap(), small);
        let large = encode_image(
            &DynamicImage::new_rgba8(2400, 1200),
            Some(OutputFormat::Png),
        )
        .unwrap();
        let processed = prepare_inline_image(&large).unwrap();
        assert!(processed.data.len() < INLINE_IMAGE_MAX_BASE64_BYTES);
        let resized = image::load_from_memory(&STANDARD.decode(processed.data).unwrap()).unwrap();
        assert_eq!((resized.width(), resized.height()), (2000, 1000));
        assert_eq!(
            processed.hints,
            vec![
                "[Image: original 2400x1200, displayed at 2000x1000. Multiply coordinates by 1.20 to map to original image.]"
                    .to_string()
            ]
        );
    }

    #[test]
    fn inline_bmp_is_converted_and_invalid_images_are_reported() {
        let mut bmp = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(4, 4)
            .write_to(&mut bmp, ImageFormat::Bmp)
            .unwrap();
        let processed = prepare_inline_image(&bmp.into_inner()).unwrap();
        assert_eq!(processed.mime_type, "image/png");
        let mut blocks = vec![ice_ai::types::Content::Image(ice_ai::types::ImageContent {
            kind: ice_ai::types::ImageContentType,
            data: "invalid".into(),
            mime_type: "image/png".into(),
        })];
        normalize_image_blocks(&mut blocks);
        assert!(
            matches!(&blocks[0], ice_ai::types::Content::Text(text) if text.text.contains("Image omitted"))
        );
    }

    #[test]
    fn inline_images_reduce_encoded_payload_even_below_dimension_limit() {
        let mut rgba = image::RgbaImage::new(1200, 1200);
        let mut seed = 1u32;
        for pixel in rgba.pixels_mut() {
            for channel in &mut pixel.0 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *channel = seed as u8;
            }
        }
        let png = encode_image(&DynamicImage::ImageRgba8(rgba), Some(OutputFormat::Png)).unwrap();
        assert!(png.len().div_ceil(3) * 4 > INLINE_IMAGE_MAX_BASE64_BYTES);
        let processed = prepare_inline_image(&png).unwrap();
        assert!(processed.data.len() < INLINE_IMAGE_MAX_BASE64_BYTES);
        assert_eq!(processed.mime_type, "image/jpeg");
    }

    #[test]
    fn fit_within_constrains_width_then_height() {
        assert_eq!(fit_within((4000, 1000), 2000, 2000), (2000, 500));
        assert_eq!(fit_within((1000, 3000), 2000, 2000), (667, 2000));
        assert_eq!(fit_within((10, 10), 2000, 2000), (10, 10));
    }

    #[test]
    fn bmp_without_auto_resize_reports_the_conversion() {
        use crate::tools::read::ReadImageProcessor;
        let mut bmp = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(4, 4)
            .write_to(&mut bmp, ImageFormat::Bmp)
            .unwrap();
        let processed = InlineImageProcessor
            .process(&bmp.into_inner(), "image/bmp", false)
            .unwrap();
        assert_eq!(processed.mime_type, "image/png");
        assert_eq!(
            processed.hints,
            vec!["[Image converted from image/bmp to image/png.]".to_string()]
        );
    }

    #[test]
    fn test_detect_image_mime_png() {
        let png_header = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_image_mime(&png_header), Some("image/png"));
    }

    #[test]
    fn test_detect_image_mime_jpeg() {
        let jpeg_header = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert_eq!(detect_image_mime(&jpeg_header), Some("image/jpeg"));
    }

    #[test]
    fn test_detect_image_mime_unknown() {
        let unknown = vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        assert_eq!(detect_image_mime(&unknown), None);
    }

    #[test]
    fn test_detect_image_mime_too_short() {
        let short = vec![0x89, 0x50];
        assert_eq!(detect_image_mime(&short), None);
    }
}
