use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("invalid frame: {0}")]
    InvalidFrame(&'static str),
}
