use thiserror::Error;

#[derive(Error, Debug)]
pub enum SandError {
    #[error("Database error: {0}")]
    Database(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Model loading error: {0}")]
    ModelLoading(String),

    #[error("Tokenizer error: {0}")]
    Tokenizer(String),
}

pub type Result<T> = std::result::Result<T, SandError>;
