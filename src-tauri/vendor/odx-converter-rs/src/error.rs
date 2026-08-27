// src/error.rs – Typed errors for the ODX converter.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OdxError {
    #[error("XML parse error: {0}")]
    ParseError(String),

    #[error("ODXLINK resolution failed: {0}")]
    ResolutionError(String),

    #[error("SNREF resolution failed: {0}")]
    SnRefError(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("XML parse error: {0}")]
    Xml(#[from] roxmltree::Error),
}
