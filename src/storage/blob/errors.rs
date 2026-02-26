// src/storage/blob/errors.rs
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(thiserror::Error, Debug)]
pub enum VardaStorageError {
    #[error("File not found")]
    FileNotFound,
    #[error("Wrong offset")]
    WrongOffset,
    #[error("File already exists")]
    FileAlreadyExists,
    #[error("Unknown error: {0}")]
    Unknown(#[from] anyhow::Error),
    #[error("Storage error: {0}")]
    StorageError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl IntoResponse for VardaStorageError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::FileNotFound => (StatusCode::NOT_FOUND, "Not Found".to_string()),
            Self::WrongOffset => (StatusCode::CONFLICT, "Wrong offset".to_string()),
            Self::FileAlreadyExists => (StatusCode::CONFLICT, "File already exists".to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, message).into_response()
    }
}
