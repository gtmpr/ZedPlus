// Phase 13a: Multimodal Inputs (Vision, PDF, Files)
// 
// Supports:
// - Image attachments (base64 encoding for API transmission)
// - PDF documents (native pass-through for Gemini, base64 for Claude, text extraction for Ollama)
// - CSV/plain text files (injected as fenced context blocks)
// - Vision capability detection and auto-routing to vision-capable models

use std::path::Path;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub path: std::path::PathBuf,
    pub media_type: MediaType,
    pub data: AttachmentData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaType {
    ImageJpeg,
    ImagePng,
    ImageGif,
    ImageWebp,
    PdfDocument,
    PlainText,
    Csv,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttachmentData {
    /// Base64-encoded binary data (for images and PDFs)
    Base64(String),
    /// Plain text content
    Text(String),
}

impl Attachment {
    /// Load an attachment from disk and prepare it for transmission
    pub async fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        
        // Detect media type from extension
        let media_type = detect_media_type(path)?;
        
        // Read file content
        let content = tokio::fs::read(path).await?;
        
        // Prepare data based on type
        let data = match media_type {
            MediaType::PlainText | MediaType::Csv => {
                let text = String::from_utf8(content)?;
                AttachmentData::Text(text)
            }
            MediaType::ImageJpeg
            | MediaType::ImagePng
            | MediaType::ImageGif
            | MediaType::ImageWebp
            | MediaType::PdfDocument => {
                let b64 = base64_encode(&content);
                AttachmentData::Base64(b64)
            }
        };
        
        Ok(Attachment {
            path: path.to_path_buf(),
            media_type,
            data,
        })
    }
    
    /// Get the MIME type for API transmission
    pub fn mime_type(&self) -> &'static str {
        match self.media_type {
            MediaType::ImageJpeg => "image/jpeg",
            MediaType::ImagePng => "image/png",
            MediaType::ImageGif => "image/gif",
            MediaType::ImageWebp => "image/webp",
            MediaType::PdfDocument => "application/pdf",
            MediaType::PlainText => "text/plain",
            MediaType::Csv => "text/csv",
        }
    }
    
    /// Format attachment as context for injection into prompts
    pub fn as_context_block(&self) -> Result<String> {
        match &self.data {
            AttachmentData::Text(text) => {
                let media_desc = match self.media_type {
                    MediaType::PlainText => "plaintext file",
                    MediaType::Csv => "CSV file",
                    _ => "file",
                };
                
                Ok(format!(
                    "```{}\n{}\n```",
                    media_desc,
                    text
                ))
            }
            AttachmentData::Base64(_) => {
                Err(anyhow!(
                    "Cannot convert binary attachment to text context block. \
                     This attachment should be passed to a vision-capable backend."
                ))
            }
        }
    }
}

/// Detect media type from file extension
fn detect_media_type(path: &Path) -> Result<MediaType> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .ok_or_else(|| anyhow!("No file extension found"))?;
    
    match ext.as_str() {
        "jpg" | "jpeg" => Ok(MediaType::ImageJpeg),
        "png" => Ok(MediaType::ImagePng),
        "gif" => Ok(MediaType::ImageGif),
        "webp" => Ok(MediaType::ImageWebp),
        "pdf" => Ok(MediaType::PdfDocument),
        "txt" => Ok(MediaType::PlainText),
        "csv" => Ok(MediaType::Csv),
        _ => Err(anyhow!("Unsupported file type: .{}", ext)),
    }
}

/// Base64 encode binary data
fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write as FmtWrite;
    
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    
    for chunk in data.chunks(3) {
        let b1 = chunk[0];
        let b2 = chunk.get(1).copied().unwrap_or(0);
        let b3 = chunk.get(2).copied().unwrap_or(0);
        
        let n = ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);
        
        result.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        result.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        
        if chunk.len() > 1 {
            result.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        
        if chunk.len() > 2 {
            result.push(TABLE[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    
    result
}

/// Check if a model supports vision capabilities
pub fn model_supports_vision(model_id: &str) -> bool {
    // Vision-capable models (from models.toml metadata)
    matches!(
        model_id,
        "claude-opus"
            | "claude-sonnet"
            | "claude-haiku"
            | "gemini-pro-vision"
            | "gemini-pro"
            | "gemini-flash"
            | "gpt-4-vision"
            | "gpt-4o"
            | "gpt-4o-mini"
    )
}

/// Check if a model supports PDF documents
pub fn model_supports_pdf(model_id: &str) -> bool {
    // Gemini supports PDF natively; Claude can handle base64
    matches!(
        model_id,
        "claude-opus" | "claude-sonnet" | "claude-haiku" | "gemini-pro" | "gemini-flash"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_media_type_detection() {
        assert!(matches!(
            detect_media_type(Path::new("image.jpg")).unwrap(),
            MediaType::ImageJpeg
        ));
        assert!(matches!(
            detect_media_type(Path::new("doc.pdf")).unwrap(),
            MediaType::PdfDocument
        ));
        assert!(detect_media_type(Path::new("file")).is_err());
    }
    
    #[test]
    fn test_base64_encode() {
        let data = b"hello";
        let encoded = base64_encode(data);
        assert_eq!(encoded, "aGVsbG8=");
    }
    
    #[test]
    fn test_vision_detection() {
        assert!(model_supports_vision("claude-opus"));
        assert!(model_supports_vision("gemini-pro"));
        assert!(!model_supports_vision("claude-next")); // hypothetical non-vision model
    }
}
