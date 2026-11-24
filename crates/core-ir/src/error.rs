use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("track not found: {0}")]
    TrackNotFound(String),

    #[error("clip not found: {0}")]
    ClipNotFound(String),

    #[error("media asset not found: {0}")]
    MediaNotFound(String),

    #[error("track '{track_id}' is locked by agent '{holder}'")]
    TrackLocked { track_id: String, holder: String },

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("nothing to undo")]
    NothingToUndo,

    #[error("nothing to redo")]
    NothingToRedo,

    #[error("persistence error: {0}")]
    Persistence(String),
}
