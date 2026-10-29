use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConvertError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Corrupted file: {0}")]
    CorruptedFile(String),

    #[error("Image extraction failed: {0}")]
    ImageExtractionFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Transcription error: {0}")]
    TranscriptionError(String),

    #[error(transparent)]
    IoError(#[from] std::io::Error),
}
