use crate::error::ConvertError;
use crate::model::document::{Document, Element, Metadata, Page, RichText, TextSegment};
use crate::model::options::ConvertOptions;
use base64::Engine as _;
use std::path::Path;
use std::process::Command;
use tracing::{debug, info};

pub struct ImageOcrConverter;

/// OCR engine selection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum OcrEngine {
    /// Local Tesseract CLI (requires `tesseract` installed).
    #[default]
    Local,
    /// Cloud OCR via OpenAI Vision API (requires `OPENAI_API_KEY`).
    Cloud,
}

impl ImageOcrConverter {
    /// Convert with a specific OCR engine.
    pub fn convert_with_engine(
        input: &Path,
        _options: &ConvertOptions,
        engine: OcrEngine,
    ) -> Result<Document, ConvertError> {
        info!(input = %input.display(), ?engine, "Image OCR conversion started");

        if !input.exists() {
            return Err(ConvertError::FileNotFound(input.to_path_buf()));
        }

        let raw_text = match engine {
            OcrEngine::Local => ocr_local(input)?,
            OcrEngine::Cloud => ocr_cloud(input)?,
        };

        debug!(text_len = raw_text.len(), "OCR text extracted");

        let paragraphs = split_paragraphs(&raw_text);
        debug!(paragraph_count = paragraphs.len(), "Paragraphs split");

        let elements: Vec<Element> = paragraphs
            .into_iter()
            .map(|text| Element::Paragraph {
                text: plain_rich_text(text),
            })
            .collect();

        let title = input
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());

        let doc = Document {
            metadata: Metadata {
                title,
                author: None,
                date: None,
            },
            pages: vec![Page { elements }],
        };

        info!(
            elements = doc.pages.first().map_or(0, |p| p.elements.len()),
            "Image OCR conversion complete"
        );
        Ok(doc)
    }
}

/// Run OCR via the local `tesseract` CLI command.
fn ocr_local(input: &Path) -> Result<String, ConvertError> {
    debug!("Running local Tesseract CLI");

    let output = Command::new("tesseract")
        .arg(input.as_os_str())
        .arg("stdout")
        .arg("-l")
        .arg("eng")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConvertError::ImageExtractionFailed(
                    "Tesseract not found. Install it: brew install tesseract (macOS) or apt install tesseract-ocr (Linux)".to_string(),
                )
            } else {
                ConvertError::ImageExtractionFailed(format!("Failed to run tesseract: {e}"))
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ConvertError::ImageExtractionFailed(format!(
            "Tesseract failed: {stderr}"
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run OCR via OpenAI Vision API.
fn ocr_cloud(input: &Path) -> Result<String, ConvertError> {
    debug!("Running cloud OCR via OpenAI Vision API");

    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        ConvertError::ImageExtractionFailed(
            "OPENAI_API_KEY environment variable not set (required for --engine cloud)".to_string(),
        )
    })?;

    let file_size = std::fs::metadata(input)?.len();
    if file_size > 20 * 1024 * 1024 {
        return Err(ConvertError::ImageExtractionFailed(format!(
            "File size ({} bytes) exceeds OpenAI 20MB limit",
            file_size
        )));
    }

    let image_bytes = std::fs::read(input)?;
    let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_bytes);

    let extension = input.extension().and_then(|e| e.to_str()).unwrap_or("png");
    let mime_type = match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        _ => "image/png",
    };

    let body = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "Extract all text from this image. Return only the extracted text, preserving paragraph structure. Do not add any commentary."
                },
                {
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{mime_type};base64,{base64_image}")
                    }
                }
            ]
        }],
        "max_tokens": 4096
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| ConvertError::NetworkError(format!("Failed to build HTTP client: {e}")))?;
    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .map_err(|e| ConvertError::NetworkError(format!("OpenAI API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(ConvertError::NetworkError(format!(
            "OpenAI API error {status}: {body}"
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .map_err(|e| ConvertError::NetworkError(format!("Failed to parse API response: {e}")))?;

    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| {
            ConvertError::NetworkError(
                "OpenAI API response missing choices[0].message.content".to_string(),
            )
        })?
        .to_string();

    Ok(text)
}

/// Splits raw OCR text into paragraphs by double newlines or runs of whitespace-only lines.
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.trim().to_string());
                current.clear();
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(trimmed);
        }
    }

    if !current.is_empty() {
        paragraphs.push(current.trim().to_string());
    }

    paragraphs
}

/// Creates a plain RichText with no formatting (no bold, italic, or code).
fn plain_rich_text(text: String) -> RichText {
    RichText {
        segments: vec![TextSegment {
            text,
            bold: false,
            italic: false,
            code: false,
            link: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_not_found() {
        let opts = ConvertOptions::default();
        let result = ImageOcrConverter::convert_with_engine(
            Path::new("nonexistent.png"),
            &opts,
            OcrEngine::Local,
        );
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConvertError::FileNotFound(_)));
    }

    #[test]
    fn test_split_paragraphs_basic() {
        let text = "Hello world\nthis is line two\n\nSecond paragraph here\n";
        let result = split_paragraphs(text);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "Hello world this is line two");
        assert_eq!(result[1], "Second paragraph here");
    }

    #[test]
    fn test_split_paragraphs_multiple_blank_lines() {
        let text = "First\n\n\n\nSecond\n\nThird";
        let result = split_paragraphs(text);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "First");
        assert_eq!(result[1], "Second");
        assert_eq!(result[2], "Third");
    }

    #[test]
    fn test_split_paragraphs_empty_input() {
        let result = split_paragraphs("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_paragraphs_whitespace_only() {
        let result = split_paragraphs("   \n  \n   ");
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_paragraphs_single_line() {
        let result = split_paragraphs("Just one line");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Just one line");
    }

    #[test]
    fn test_plain_rich_text() {
        let rt = plain_rich_text("hello".to_string());
        assert_eq!(rt.segments.len(), 1);
        assert_eq!(rt.segments[0].text, "hello");
        assert!(!rt.segments[0].bold);
        assert!(!rt.segments[0].italic);
        assert!(!rt.segments[0].code);
        assert!(rt.segments[0].link.is_none());
    }
}
