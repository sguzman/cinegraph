use thiserror::Error;

pub type Result<T> = std::result::Result<T, CinegraphError>;

#[derive(Debug, Error)]
pub enum CinegraphError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}
