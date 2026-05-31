use std::path::{Path, PathBuf};

use crate::config::AppConfig;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            root: PathBuf::from(&config.data.root),
        }
    }

    pub fn raw_dir(&self) -> PathBuf {
        self.root.join("raw")
    }

    pub fn raw_source_dir(&self, source: &str) -> PathBuf {
        self.raw_dir().join(source)
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs").join("sha256")
    }

    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        let prefix = &sha256[..2];
        self.blobs_dir().join(prefix).join(sha256)
    }

    pub fn db_dir(&self) -> PathBuf {
        self.root.join("db")
    }

    pub fn sqlite_path(&self, config: &AppConfig) -> PathBuf {
        PathBuf::from(&config.sqlite.path)
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    pub fn ensure_dirs(&self, config: &AppConfig) -> std::io::Result<()> {
        for path in [
            self.root.clone(),
            self.raw_dir(),
            self.raw_source_dir("imdb"),
            self.blobs_dir(),
            self.db_dir(),
            self.logs_dir(),
            self.temp_dir(),
            self.root.join("index").join("tantivy"),
            self.root.join("graph").join("oxigraph"),
        ] {
            std::fs::create_dir_all(path)?;
        }

        if let Some(parent) = Path::new(&config.sqlite.path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AppConfig, DataConfig, FetchConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig,
        SqliteConfig,
    };
    use tempfile::tempdir;

    #[test]
    fn blob_path_uses_prefix_directory() {
        let paths = AppPaths {
            root: PathBuf::from(".data"),
        };
        assert_eq!(
            paths.blob_path("abcdef"),
            PathBuf::from(".data/blobs/sha256/ab/abcdef")
        );
    }

    #[test]
    fn ensure_dirs_creates_expected_layout() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join(".data");
        let config = AppConfig {
            data: DataConfig {
                root: root.to_string_lossy().to_string(),
            },
            sqlite: SqliteConfig {
                path: root
                    .join("db/cinegraph.sqlite")
                    .to_string_lossy()
                    .to_string(),
                max_connections: 1,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
                file: root
                    .join("logs/cinegraph.log")
                    .to_string_lossy()
                    .to_string(),
            },
            fetch: FetchConfig {
                user_agent: "cinegraph-test".to_string(),
                connect_timeout_seconds: 5,
                request_timeout_seconds: 5,
                retries: 1,
            },
            sources: SourcesConfig {
                imdb: ImdbSourceConfig {
                    enabled: true,
                    base_url: "http://localhost/".to_string(),
                    datasets: vec!["name.basics.tsv.gz".to_string()],
                },
            },
        };
        let paths = AppPaths::from_config(&config);
        paths.ensure_dirs(&config).expect("ensure dirs");

        assert!(paths.raw_source_dir("imdb").exists());
        assert!(paths.blobs_dir().exists());
        assert!(paths.db_dir().exists());
        assert!(paths.logs_dir().exists());
        assert!(root.join("index/tantivy").exists());
        assert!(root.join("graph/oxigraph").exists());
    }
}
