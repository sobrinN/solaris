use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("OpenRouter API key is not configured")]
    NotConfigured,

    #[error("OpenRouter HTTP {status}: {message}")]
    HttpStatus {
        status: u16,
        message: String,
        retry_after_secs: Option<u64>,
    },

    #[error("OpenRouter stream error: {0}")]
    StreamError(String),

    #[error("HTTP transport error: {0}")]
    Transport(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("tool error: {0}")]
    ToolError(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("backend error: {0}")]
    Backend(String),
}

impl AgentError {
    pub fn is_retryable_status(status: u16) -> bool {
        matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 520..=524)
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::HttpStatus { status, .. } => Self::is_retryable_status(*status),
            Self::Transport(_) => true,
            _ => false,
        }
    }
}
