use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

use crate::{CinegraphError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub data: DataConfig,
    pub sqlite: SqliteConfig,
    pub logging: LoggingConfig,
    pub graph: GraphConfig,
    pub fetch: FetchConfig,
    pub sources: SourcesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataConfig {
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteConfig {
    pub path: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphConfig {
    pub base_iri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchConfig {
    pub user_agent: String,
    pub connect_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcesConfig {
    pub imdb: ImdbSourceConfig,
    pub tmdb: TmdbSourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImdbSourceConfig {
    pub enabled: bool,
    pub base_url: String,
    pub datasets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbSourceConfig {
    pub enabled: bool,
    pub export_base_url: String,
    pub api_base_url: String,
    pub api_read_access_token: String,
    pub language: String,
    pub hydrate_limit_per_run: u32,
    pub request_interval_ms: u64,
    pub export_days_back: u32,
    pub include_adult: bool,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let config = toml::from_str(&raw)?;
        Ok(config)
    }

    pub fn from_example() -> Result<Self> {
        Self::load(Path::new("config/cinegraph.example.toml"))
    }

    pub fn validate(&self) -> Result<()> {
        if self.sources.imdb.datasets.is_empty() {
            return Err(CinegraphError::Config(
                "sources.imdb.datasets must not be empty".to_string(),
            ));
        }
        if self.sources.tmdb.enabled
            && (self.sources.tmdb.export_base_url.is_empty()
                || self.sources.tmdb.api_base_url.is_empty())
        {
            return Err(CinegraphError::Config(
                "sources.tmdb URLs must not be empty when TMDb is enabled".to_string(),
            ));
        }
        if !self.graph.base_iri.ends_with('/') {
            return Err(CinegraphError::Config(
                "graph.base_iri must end with '/'".to_string(),
            ));
        }
        Ok(())
    }
}
