use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("ffmpeg/ffprobe binary unavailable: {0}")]
    ToolchainUnavailable(String),

    #[error("process execution failed: {0}")]
    ProcessFailed(String),

    #[error("failed to parse media output: {0}")]
    ParseError(String),

    #[error("storage access denied or invalid path: {0}")]
    StorageError(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
