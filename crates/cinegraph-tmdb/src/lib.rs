use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    time::Duration,
};

use cinegraph_core::{AppConfig, CinegraphError, Result};
use cinegraph_db::{Database, models::DownloadArtifact, queries};
use flate2::read::MultiGzDecoder;
use reqwest::{StatusCode, header};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::info;

pub const EXPORT_IMPORTER_NAME: &str = "tmdb-export";
pub const EXPORT_IMPORTER_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize)]
pub struct TmdbImportStats {
    pub export_rows_seen: i64,
    pub export_rows_imported: i64,
    pub export_rows_skipped: i64,
    pub movies_hydrated: i64,
    pub credits_imported: i64,
    pub title_links_created: i64,
    pub hydration_failures: i64,
}

pub struct TmdbImporter<'a> {
    db: &'a Database,
    client: reqwest::Client,
}

impl<'a> TmdbImporter<'a> {
    pub fn new(db: &'a Database, config: &AppConfig) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        if !config.sources.tmdb.api_read_access_token.is_empty() {
            let value = format!("Bearer {}", config.sources.tmdb.api_read_access_token);
            headers.insert(
                header::AUTHORIZATION,
                value
                    .parse()
                    .map_err(|error| CinegraphError::Other(format!("invalid TMDb token header: {error}")))?,
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(&config.fetch.user_agent)
            .connect_timeout(Duration::from_secs(config.fetch.connect_timeout_seconds))
            .timeout(Duration::from_secs(config.fetch.request_timeout_seconds))
            .build()?;

        Ok(Self { db, client })
    }

    pub async fn import_latest(&self, config: &AppConfig) -> Result<TmdbImportStats> {
        let Some((_, artifact)) = queries::latest_artifact_for_source(self.db.pool(), "tmdb").await? else {
            return Err(CinegraphError::Other(
                "no TMDb export artifact found; run `cinegraph fetch tmdb` first".to_string(),
            ));
        };

        let mut stats = self.import_export_artifact(&artifact).await?;
        let hydration = self.hydrate_pending(config, artifact.id).await?;
        stats.movies_hydrated += hydration.movies_hydrated;
        stats.credits_imported += hydration.credits_imported;
        stats.title_links_created += hydration.title_links_created;
        stats.hydration_failures += hydration.hydration_failures;
        Ok(stats)
    }

    async fn import_export_artifact(&self, artifact: &DownloadArtifact) -> Result<TmdbImportStats> {
        let started = queries::try_begin_import_run(
            self.db.pool(),
            artifact.id,
            EXPORT_IMPORTER_NAME,
            EXPORT_IMPORTER_VERSION,
        )
        .await?;

        if !started {
            return Ok(TmdbImportStats {
                export_rows_skipped: 1,
                ..TmdbImportStats::default()
            });
        }

        let result = self
            .import_export_rows(artifact.id, Path::new(&artifact.local_path))
            .await;
        match result {
            Ok(stats) => {
                queries::finish_import_run(
                    self.db.pool(),
                    artifact.id,
                    EXPORT_IMPORTER_NAME,
                    EXPORT_IMPORTER_VERSION,
                    "completed",
                    stats.export_rows_seen,
                    stats.export_rows_imported,
                    0,
                    stats.export_rows_skipped,
                    None,
                )
                .await?;
                Ok(stats)
            }
            Err(error) => {
                queries::finish_import_run(
                    self.db.pool(),
                    artifact.id,
                    EXPORT_IMPORTER_NAME,
                    EXPORT_IMPORTER_VERSION,
                    "failed",
                    0,
                    0,
                    0,
                    0,
                    Some(&error.to_string()),
                )
                .await?;
                Err(error)
            }
        }
    }

    async fn import_export_rows(&self, export_artifact_id: i64, path: &Path) -> Result<TmdbImportStats> {
        let reader = BufReader::new(MultiGzDecoder::new(File::open(path)?));

        let mut stats = TmdbImportStats::default();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            stats.export_rows_seen += 1;
            let row: TmdbExportLine = serde_json::from_str(&line)?;
            queries::upsert_tmdb_movie_export(
                self.db.pool(),
                export_artifact_id,
                row.id,
                row.adult,
                row.original_title.as_deref(),
                row.popularity,
                row.video,
            )
            .await?;
            stats.export_rows_imported += 1;
        }

        info!(
            export_rows_seen = stats.export_rows_seen,
            export_rows_imported = stats.export_rows_imported,
            "TMDb export imported"
        );
        Ok(stats)
    }

    async fn hydrate_pending(
        &self,
        config: &AppConfig,
        export_artifact_id: i64,
    ) -> Result<TmdbImportStats> {
        if config.sources.tmdb.api_read_access_token.is_empty() {
            return Err(CinegraphError::Config(
                "sources.tmdb.api_read_access_token must be set to import TMDb hydration".to_string(),
            ));
        }

        let pending = queries::pending_tmdb_movie_hydrations(
            self.db.pool(),
            export_artifact_id,
            config.sources.tmdb.include_adult,
            config.sources.tmdb.hydrate_limit_per_run as i64,
        )
        .await?;

        let mut stats = TmdbImportStats::default();
        for movie in pending {
            match self.hydrate_movie(config, &movie).await {
                Ok(movie_stats) => {
                    stats.movies_hydrated += movie_stats.movies_hydrated;
                    stats.credits_imported += movie_stats.credits_imported;
                    stats.title_links_created += movie_stats.title_links_created;
                }
                Err(error) => {
                    queries::mark_tmdb_movie_hydration_failed(
                        self.db.pool(),
                        movie.export_artifact_id,
                        movie.tmdb_movie_id,
                        &error.to_string(),
                    )
                    .await?;
                    stats.hydration_failures += 1;
                }
            }
        }
        Ok(stats)
    }

    async fn hydrate_movie(
        &self,
        config: &AppConfig,
        movie: &cinegraph_db::models::PendingTmdbMovieHydration,
    ) -> Result<TmdbImportStats> {
        let response = self
            .get_movie_details(config, movie.tmdb_movie_id)
            .await?;

        let raw_json = serde_json::to_string(&response)?;
        queries::upsert_tmdb_movie(
            self.db.pool(),
            response.id,
            response
                .external_ids
                .as_ref()
                .and_then(|ids| ids.imdb_id.as_deref())
                .or(response.imdb_id.as_deref()),
            &response.title,
            response.original_title.as_deref(),
            response.original_language.as_deref(),
            response.overview.as_deref(),
            response.release_date.as_deref(),
            response.runtime,
            response.status.as_deref(),
            response.popularity,
            response.vote_average,
            response.vote_count,
            &raw_json,
        )
        .await?;

        queries::clear_tmdb_movie_credits(self.db.pool(), response.id).await?;
        let mut stats = TmdbImportStats {
            movies_hydrated: 1,
            ..TmdbImportStats::default()
        };

        if let Some(credits) = response.credits {
            for cast in credits.cast {
                queries::upsert_tmdb_person(self.db.pool(), cast.id, &cast.name).await?;
                queries::replace_tmdb_movie_credit(
                    self.db.pool(),
                    response.id,
                    cast.id,
                    &format!("cast:{}:{}:{}", cast.id, cast.order.unwrap_or_default(), cast.character.clone().unwrap_or_default()),
                    cast.order,
                    "cast",
                    cast.known_for_department.as_deref(),
                    None,
                    cast.character.as_deref(),
                    &serde_json::to_string(&cast)?,
                )
                .await?;
                stats.credits_imported += 1;
            }

            for crew in credits.crew {
                queries::upsert_tmdb_person(self.db.pool(), crew.id, &crew.name).await?;
                queries::replace_tmdb_movie_credit(
                    self.db.pool(),
                    response.id,
                    crew.id,
                    &format!("crew:{}:{}:{}", crew.id, crew.department.clone().unwrap_or_default(), crew.job.clone().unwrap_or_default()),
                    None,
                    "crew",
                    crew.department.as_deref(),
                    crew.job.as_deref(),
                    None,
                    &serde_json::to_string(&crew)?,
                )
                .await?;
                stats.credits_imported += 1;
            }
        }

        let imdb_id = response
            .external_ids
            .as_ref()
            .and_then(|ids| ids.imdb_id.as_deref())
            .or(response.imdb_id.as_deref());
        if let Some(imdb_id) = imdb_id {
            if queries::title_exists(self.db.pool(), imdb_id).await? {
                if queries::link_title_to_tmdb_movie(self.db.pool(), imdb_id, response.id).await? {
                    stats.title_links_created += 1;
                }
            }
        }

        queries::mark_tmdb_movie_hydrated(self.db.pool(), movie.export_artifact_id, movie.tmdb_movie_id).await?;

        if config.sources.tmdb.request_interval_ms > 0 {
            sleep(Duration::from_millis(config.sources.tmdb.request_interval_ms)).await;
        }

        Ok(stats)
    }

    async fn get_movie_details(
        &self,
        config: &AppConfig,
        tmdb_movie_id: i64,
    ) -> Result<TmdbMovieDetails> {
        let url = format!(
            "{}/movie/{}?append_to_response=credits,external_ids&language={}",
            config.sources.tmdb.api_base_url, tmdb_movie_id, config.sources.tmdb.language
        );

        let response = self.client.get(&url).send().await?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(1);
            sleep(Duration::from_secs(retry_after)).await;
            let retry = self.client.get(&url).send().await?;
            return retry.json().await.map_err(Into::into);
        }
        response.error_for_status_ref()?;
        Ok(response.json().await?)
    }
}

impl Default for TmdbImportStats {
    fn default() -> Self {
        Self {
            export_rows_seen: 0,
            export_rows_imported: 0,
            export_rows_skipped: 0,
            movies_hydrated: 0,
            credits_imported: 0,
            title_links_created: 0,
            hydration_failures: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TmdbExportLine {
    id: i64,
    adult: bool,
    original_title: Option<String>,
    popularity: Option<f64>,
    video: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct TmdbMovieDetails {
    id: i64,
    title: String,
    original_title: Option<String>,
    original_language: Option<String>,
    overview: Option<String>,
    release_date: Option<String>,
    runtime: Option<i64>,
    status: Option<String>,
    popularity: Option<f64>,
    vote_average: Option<f64>,
    vote_count: Option<i64>,
    imdb_id: Option<String>,
    external_ids: Option<TmdbExternalIds>,
    credits: Option<TmdbCredits>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TmdbExternalIds {
    imdb_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TmdbCredits {
    cast: Vec<TmdbCastCredit>,
    crew: Vec<TmdbCrewCredit>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TmdbCastCredit {
    id: i64,
    name: String,
    character: Option<String>,
    order: Option<i64>,
    known_for_department: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TmdbCrewCredit {
    id: i64,
    name: String,
    department: Option<String>,
    job: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinegraph_core::{
        AppPaths,
        config::{
            DataConfig, FetchConfig, GraphConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig,
            SqliteConfig, TmdbSourceConfig,
        },
    };
    use httpmock::{Method::GET, MockServer};
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn import_and_hydrate_tmdb_export() {
        let server = MockServer::start();
        let movie = server.mock(|when, then| {
            when.method(GET)
                .path("/3/movie/11")
                .query_param("append_to_response", "credits,external_ids")
                .query_param("language", "en-US")
                .header("authorization", "Bearer test-token");
            then.status(200).json_body_obj(&serde_json::json!({
                "id": 11,
                "title": "Seven Samurai",
                "original_title": "Shichinin no samurai",
                "original_language": "ja",
                "overview": "A village hires seven samurai.",
                "release_date": "1954-04-26",
                "runtime": 207,
                "status": "Released",
                "popularity": 22.5,
                "vote_average": 8.5,
                "vote_count": 1000,
                "external_ids": { "imdb_id": "tt0047478" },
                "credits": {
                    "cast": [
                        { "id": 101, "name": "Toshiro Mifune", "character": "Kikuchiyo", "order": 0, "known_for_department": "Acting" }
                    ],
                    "crew": [
                        { "id": 201, "name": "Akira Kurosawa", "department": "Directing", "job": "Director" }
                    ]
                }
            }));
        });

        let temp = tempdir().expect("tempdir");
        let root = temp.path().join(".data");
        let config = AppConfig {
            data: DataConfig {
                root: root.to_string_lossy().to_string(),
            },
            sqlite: SqliteConfig {
                path: root.join("db/cinegraph.sqlite").to_string_lossy().to_string(),
                max_connections: 1,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
                file: root.join("logs/cinegraph.log").to_string_lossy().to_string(),
            },
            graph: GraphConfig {
                base_iri: "https://cinegraph.local/".to_string(),
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
                    datasets: vec!["title.basics.tsv.gz".to_string()],
                },
                tmdb: TmdbSourceConfig {
                    enabled: true,
                    export_base_url: "http://localhost/exports/".to_string(),
                    api_base_url: format!("{}/3", server.base_url()),
                    api_read_access_token: "test-token".to_string(),
                    language: "en-US".to_string(),
                    hydrate_limit_per_run: 10,
                    request_interval_ms: 0,
                    export_days_back: 2,
                    include_adult: false,
                },
            },
        };
        let paths = AppPaths::from_config(&config);
        paths.ensure_dirs(&config).expect("dirs");
        let db = Database::connect(&config, &paths).await.expect("db");
        db.migrate().await.expect("migrate");
        sqlx::query("INSERT INTO titles (imdb_id, title_type, primary_title, original_title, is_adult, start_year) VALUES ('tt0047478', 'movie', 'Seven Samurai', 'Shichinin no samurai', 0, 1954)")
            .execute(db.pool())
            .await
            .expect("seed title");

        let export_path = paths.raw_source_dir("tmdb").join("movie_ids_05_30_2026.json.gz");
        let export_file = File::create(&export_path).expect("export file");
        let mut encoder = flate2::write::GzEncoder::new(export_file, flate2::Compression::default());
        encoder
            .write_all(br#"{"id":11,"adult":false,"original_title":"Seven Samurai","popularity":22.5,"video":false}"#)
            .expect("write export");
        encoder.write_all(b"\n").expect("newline");
        encoder.finish().expect("finish gzip");

        let dataset = queries::upsert_dataset(
            db.pool(),
            "tmdb",
            "movie_ids_05_30_2026.json.gz",
            "http://localhost/exports/movie_ids_05_30_2026.json.gz",
        )
        .await
        .expect("dataset");
        let artifact = queries::insert_artifact(
            db.pool(),
            dataset.id,
            "http://localhost/exports/movie_ids_05_30_2026.json.gz",
            &export_path.to_string_lossy(),
            "hash-tmdb",
            1,
            None,
            None,
        )
        .await
        .expect("artifact");

        let importer = TmdbImporter::new(&db, &config).expect("importer");
        let stats = importer.import_latest(&config).await.expect("import");
        assert_eq!(stats.export_rows_seen, 1);
        assert_eq!(stats.movies_hydrated, 1);
        assert_eq!(stats.credits_imported, 2);
        assert_eq!(stats.title_links_created, 1);

        let linked: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM title_tmdb_links WHERE imdb_id = 'tt0047478'")
            .fetch_one(db.pool())
            .await
            .expect("link count");
        assert_eq!(linked.0, 1);

        let second = importer.import_latest(&config).await.expect("repeat import");
        assert_eq!(second.movies_hydrated, 0);
        movie.assert_hits(1);

        let export_row = queries::tmdb_movie_export_by_artifact_and_id(db.pool(), artifact.id, 11)
            .await
            .expect("export row")
            .expect("present");
        assert_eq!(export_row.hydrate_status.as_deref(), Some("completed"));
    }
}
