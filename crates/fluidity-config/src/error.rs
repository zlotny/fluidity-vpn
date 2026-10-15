use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("line {line}: unterminated quote")]
    UnterminatedQuote { line: usize },
    #[error("line {line}: <{tag}> block is not closed")]
    UnterminatedInline { line: usize, tag: String },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("referenced file not found: {0}")]
    MissingFile(String),
    #[error("path escapes the bundle: {0}")]
    UnsafePath(String),
    #[error("no OpenVPN configuration found")]
    NoConfig,
    #[error("unrecognised archive format")]
    UnknownFormat,
    #[error("configuration is not valid UTF-8")]
    NotUtf8,
}
