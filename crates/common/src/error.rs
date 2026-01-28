use thiserror::Error;

#[derive(Debug, Error)]
pub enum BtProxyError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("authentication error: {0}")]
    Auth(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, BtProxyError>;
