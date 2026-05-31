use cinegraph_core::{AppConfig, AppPaths, Result};
use cinegraph_db::{Database, models::DownloadArtifact, queries};
use futures_util::StreamExt;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncWriteExt, BufWriter},
};
use tracing::{info, info_span, instrument};
use uuid::Uuid;

pub struct Fetcher {
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub dataset_name: String,
    pub artifact: DownloadArtifact,
    pub changed: bool,
}

impl Fetcher {
    pub fn new(config: &AppConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(&config.fetch.user_agent)
            .connect_timeout(std::time::Duration::from_secs(
                config.fetch.connect_timeout_seconds,
            ))
            .timeout(std::time::Duration::from_secs(
                config.fetch.request_timeout_seconds,
            ))
            .build()?;
        Ok(Self { client })
    }

    #[instrument(skip_all, fields(source = "imdb", dataset = %dataset_name))]
    pub async fn fetch_dataset(
        &self,
        db: &Database,
        config: &AppConfig,
        paths: &AppPaths,
        dataset_name: &str,
    ) -> Result<FetchOutcome> {
        let url = format!("{}{}", config.sources.imdb.base_url, dataset_name);
        let dataset = queries::upsert_dataset(db.pool(), "imdb", dataset_name, &url).await?;
        let previous = queries::last_artifact_for_dataset(db.pool(), dataset.id).await?;
        let span = info_span!("fetch.http_request", url = %url);
        let _guard = span.enter();

        let mut request = self.client.get(&url);
        if let Some(previous) = &previous {
            if let Some(etag) = &previous.etag {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = &previous.last_modified {
                request = request.header(IF_MODIFIED_SINCE, last_modified);
            }
        }

        let response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let artifact = previous.expect("previous artifact must exist for 304");
            return Ok(FetchOutcome {
                dataset_name: dataset_name.to_string(),
                artifact,
                changed: false,
            });
        }
        response.error_for_status_ref()?;
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);

        let temp_name = format!("{}.part", Uuid::new_v4());
        let temp_path = paths.temp_dir().join(temp_name);
        let mut writer = BufWriter::new(fs::File::create(&temp_path).await?);
        let mut hasher = Sha256::new();
        let mut byte_len: i64 = 0;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            byte_len += chunk.len() as i64;
            hasher.update(&chunk);
            writer.write_all(&chunk).await?;
        }
        writer.flush().await?;

        let sha256 = hex::encode(hasher.finalize());
        let blob_path = paths.blob_path(&sha256);
        if let Some(parent) = blob_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        if fs::metadata(&blob_path).await.is_err() {
            fs::rename(&temp_path, &blob_path).await?;
        } else {
            let _ = fs::remove_file(&temp_path).await;
        }

        let friendly_path = paths.raw_source_dir("imdb").join(dataset_name);
        fs::copy(&blob_path, &friendly_path).await?;

        let local_path = blob_path.to_string_lossy().to_string();
        let artifact = if let Some(existing) =
            queries::artifact_by_hash(db.pool(), dataset.id, &sha256).await?
        {
            existing
        } else {
            queries::insert_artifact(
                db.pool(),
                dataset.id,
                &url,
                &local_path,
                &sha256,
                byte_len,
                etag.as_deref(),
                last_modified.as_deref(),
            )
            .await?
        };

        info!(sha256 = %artifact.sha256, bytes = artifact.byte_len, "dataset fetched");
        Ok(FetchOutcome {
            dataset_name: dataset_name.to_string(),
            artifact,
            changed: true,
        })
    }

    pub async fn fetch_imdb(
        &self,
        db: &Database,
        config: &AppConfig,
        paths: &AppPaths,
    ) -> Result<Vec<FetchOutcome>> {
        let mut out = Vec::new();
        for dataset_name in &config.sources.imdb.datasets {
            out.push(self.fetch_dataset(db, config, paths, dataset_name).await?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinegraph_core::config::{
        DataConfig, FetchConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig, SqliteConfig,
    };
    use httpmock::{Method::GET, MockServer};
    use tempfile::tempdir;

    #[tokio::test]
    async fn fetch_imdb_is_idempotent_for_same_hash() {
        let server = MockServer::start();
        let second_server = MockServer::start();
        let body = b"test-body";
        let first = server.mock(|when, then| {
            when.method(GET).path("/name.basics.tsv.gz");
            then.status(200)
                .header("etag", "\"abc\"")
                .header("last-modified", "Mon, 01 Jan 2024 00:00:00 GMT")
                .body(body.as_slice());
        });
        let second = second_server.mock(|when, then| {
            when.method(GET)
                .path("/name.basics.tsv.gz")
                .header("if-none-match", "\"abc\"");
            then.status(304);
        });

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
                    base_url: format!("{}/", server.base_url()),
                    datasets: vec!["name.basics.tsv.gz".to_string()],
                },
            },
        };
        let paths = AppPaths::from_config(&config);
        paths.ensure_dirs(&config).expect("dirs");
        let db = Database::connect(&config, &paths).await.expect("connect");
        db.migrate().await.expect("migrate");
        let fetcher = Fetcher::new(&config).expect("fetcher");

        let first_outcome = fetcher
            .fetch_dataset(&db, &config, &paths, "name.basics.tsv.gz")
            .await
            .expect("first fetch");

        let second_config = AppConfig {
            sources: SourcesConfig {
                imdb: ImdbSourceConfig {
                    enabled: true,
                    base_url: format!("{}/", second_server.base_url()),
                    datasets: config.sources.imdb.datasets.clone(),
                },
            },
            ..config.clone()
        };
        let second_outcome = fetcher
            .fetch_dataset(&db, &second_config, &paths, "name.basics.tsv.gz")
            .await
            .expect("second fetch");

        assert!(first_outcome.changed);
        assert!(!second_outcome.changed);
        assert_eq!(
            first_outcome.artifact.sha256,
            second_outcome.artifact.sha256
        );
        let artifact_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM download_artifacts")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(artifact_count.0, 1);
        first.assert();
        second.assert();
    }
}
