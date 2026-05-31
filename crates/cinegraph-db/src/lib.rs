pub mod models;
pub mod queries;

use cinegraph_core::{AppConfig, AppPaths, Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::str::FromStr;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(config: &AppConfig, paths: &AppPaths) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(&format!(
            "sqlite://{}",
            paths.sqlite_path(config).display()
        ))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(config.sqlite.max_connections)
            .connect_with(options)
            .await?;

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS datasets (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                dataset_name TEXT NOT NULL,
                canonical_url TEXT NOT NULL,
                license_note TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(source, dataset_name)
            );

            CREATE TABLE IF NOT EXISTS download_artifacts (
                id INTEGER PRIMARY KEY,
                dataset_id INTEGER NOT NULL REFERENCES datasets(id),
                url TEXT NOT NULL,
                local_path TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                byte_len INTEGER NOT NULL,
                etag TEXT,
                last_modified TEXT,
                fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(dataset_id, sha256)
            );

            CREATE TABLE IF NOT EXISTS import_runs (
                id INTEGER PRIMARY KEY,
                artifact_id INTEGER NOT NULL REFERENCES download_artifacts(id),
                importer_name TEXT NOT NULL,
                importer_version TEXT NOT NULL,
                started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                finished_at TEXT,
                status TEXT NOT NULL,
                rows_seen INTEGER NOT NULL DEFAULT 0,
                rows_inserted INTEGER NOT NULL DEFAULT 0,
                rows_updated INTEGER NOT NULL DEFAULT 0,
                rows_skipped INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                UNIQUE(artifact_id, importer_name, importer_version)
            );

            CREATE TABLE IF NOT EXISTS titles (
                imdb_id TEXT PRIMARY KEY,
                title_type TEXT NOT NULL,
                primary_title TEXT NOT NULL,
                original_title TEXT,
                is_adult INTEGER NOT NULL DEFAULT 0,
                start_year INTEGER,
                end_year INTEGER,
                runtime_minutes INTEGER,
                genres TEXT
            );

            CREATE TABLE IF NOT EXISTS people (
                imdb_name_id TEXT PRIMARY KEY,
                primary_name TEXT NOT NULL,
                birth_year INTEGER,
                death_year INTEGER,
                primary_professions TEXT,
                known_for_titles TEXT
            );

            CREATE TABLE IF NOT EXISTS title_ratings (
                imdb_id TEXT PRIMARY KEY REFERENCES titles(imdb_id),
                average_rating REAL,
                num_votes INTEGER
            );

            CREATE TABLE IF NOT EXISTS title_akas (
                id INTEGER PRIMARY KEY,
                imdb_id TEXT NOT NULL REFERENCES titles(imdb_id),
                ordering INTEGER,
                title TEXT NOT NULL,
                region TEXT,
                language TEXT,
                types TEXT,
                attributes TEXT,
                is_original_title INTEGER,
                UNIQUE(imdb_id, ordering, title)
            );

            CREATE TABLE IF NOT EXISTS title_crew (
                imdb_id TEXT PRIMARY KEY REFERENCES titles(imdb_id),
                directors TEXT,
                writers TEXT
            );

            CREATE TABLE IF NOT EXISTS credits (
                id INTEGER PRIMARY KEY,
                imdb_id TEXT NOT NULL REFERENCES titles(imdb_id),
                imdb_name_id TEXT NOT NULL REFERENCES people(imdb_name_id),
                ordering INTEGER,
                category TEXT,
                job TEXT,
                characters TEXT,
                source TEXT NOT NULL DEFAULT 'imdb',
                UNIQUE(imdb_id, imdb_name_id, ordering, category, source)
            );

            CREATE TABLE IF NOT EXISTS episode_edges (
                imdb_id TEXT PRIMARY KEY REFERENCES titles(imdb_id),
                parent_imdb_id TEXT NOT NULL REFERENCES titles(imdb_id),
                season_number INTEGER,
                episode_number INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_titles_primary_title ON titles(primary_title);
            CREATE INDEX IF NOT EXISTS idx_people_primary_name ON people(primary_name);
            CREATE INDEX IF NOT EXISTS idx_credits_title ON credits(imdb_id);
            CREATE INDEX IF NOT EXISTS idx_credits_person ON credits(imdb_name_id);
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinegraph_core::{
        AppConfig,
        config::{
            DataConfig, FetchConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig, SqliteConfig,
        },
    };
    use tempfile::tempdir;

    #[tokio::test]
    async fn migrate_creates_schema() {
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
        paths.ensure_dirs(&config).expect("dirs");
        let db = Database::connect(&config, &paths).await.expect("connect");
        db.migrate().await.expect("migrate");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'titles'",
        )
        .fetch_one(db.pool())
        .await
        .expect("query");
        assert_eq!(count.0, 1);
    }
}
