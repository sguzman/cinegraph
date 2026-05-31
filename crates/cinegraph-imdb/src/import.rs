use std::path::Path;

use cinegraph_core::Result;
use cinegraph_db::{Database, models::DownloadArtifact, queries};
use tracing::instrument;

use crate::{
    IMPORTER_NAME, IMPORTER_VERSION,
    rows::{
        NameBasicsRow, TitleAkasRow, TitleBasicsRow, TitleCrewRow, TitleEpisodeRow,
        TitlePrincipalsRow, TitleRatingsRow,
    },
    tsv::{imdb_null, read_gzip_tsv},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct ImportStats {
    pub rows_seen: i64,
    pub rows_inserted: i64,
    pub rows_updated: i64,
    pub rows_skipped: i64,
}

pub struct ImdbImporter<'a> {
    db: &'a Database,
}

impl<'a> ImdbImporter<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub async fn import_latest(&self) -> Result<Vec<(String, ImportStats)>> {
        let mut artifacts = queries::latest_artifacts_for_source(self.db.pool(), "imdb").await?;
        artifacts.sort_by_key(|(dataset_name, _)| dataset_priority(dataset_name));
        let mut results = Vec::new();
        for (dataset_name, artifact) in artifacts {
            let stats = self.import_artifact(&dataset_name, &artifact).await?;
            results.push((dataset_name, stats));
        }
        Ok(results)
    }

    #[instrument(skip_all, fields(dataset = %dataset_name, artifact_id = artifact.id))]
    pub async fn import_artifact(
        &self,
        dataset_name: &str,
        artifact: &DownloadArtifact,
    ) -> Result<ImportStats> {
        if !queries::try_begin_import_run(
            self.db.pool(),
            artifact.id,
            IMPORTER_NAME,
            IMPORTER_VERSION,
        )
        .await?
        {
            return Ok(ImportStats {
                rows_skipped: 1,
                ..ImportStats::default()
            });
        }

        let path = Path::new(&artifact.local_path);
        let result = match dataset_name {
            "name.basics.tsv.gz" => self.import_name_basics(path).await,
            "title.basics.tsv.gz" => self.import_title_basics(path).await,
            "title.ratings.tsv.gz" => self.import_title_ratings(path).await,
            "title.akas.tsv.gz" => self.import_title_akas(path).await,
            "title.crew.tsv.gz" => self.import_title_crew(path).await,
            "title.principals.tsv.gz" => self.import_title_principals(path).await,
            "title.episode.tsv.gz" => self.import_title_episode(path).await,
            other => Err(cinegraph_core::CinegraphError::Other(format!(
                "unsupported dataset {other}"
            ))),
        };

        match result {
            Ok(stats) => {
                queries::finish_import_run(
                    self.db.pool(),
                    artifact.id,
                    IMPORTER_NAME,
                    IMPORTER_VERSION,
                    "completed",
                    stats.rows_seen,
                    stats.rows_inserted,
                    stats.rows_updated,
                    stats.rows_skipped,
                    None,
                )
                .await?;
                Ok(stats)
            }
            Err(error) => {
                queries::finish_import_run(
                    self.db.pool(),
                    artifact.id,
                    IMPORTER_NAME,
                    IMPORTER_VERSION,
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

    async fn import_name_basics(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<NameBasicsRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO people (imdb_name_id, primary_name, birth_year, death_year, primary_professions, known_for_titles)
                VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(imdb_name_id) DO UPDATE SET
                    primary_name = excluded.primary_name,
                    birth_year = excluded.birth_year,
                    death_year = excluded.death_year,
                    primary_professions = excluded.primary_professions,
                    known_for_titles = excluded.known_for_titles
                "#,
            )
            .bind(row.imdb_name_id)
            .bind(row.primary_name)
            .bind(parse_i64(imdb_null(&row.birth_year)))
            .bind(parse_i64(imdb_null(&row.death_year)))
            .bind(imdb_null(&row.primary_professions))
            .bind(imdb_null(&row.known_for_titles))
            .execute(&mut *tx)
            .await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }

    async fn import_title_basics(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<TitleBasicsRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO titles (imdb_id, title_type, primary_title, original_title, is_adult, start_year, end_year, runtime_minutes, genres)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(imdb_id) DO UPDATE SET
                    title_type = excluded.title_type,
                    primary_title = excluded.primary_title,
                    original_title = excluded.original_title,
                    is_adult = excluded.is_adult,
                    start_year = excluded.start_year,
                    end_year = excluded.end_year,
                    runtime_minutes = excluded.runtime_minutes,
                    genres = excluded.genres
                "#,
            )
            .bind(row.imdb_id)
            .bind(row.title_type)
            .bind(row.primary_title)
            .bind(imdb_null(&row.original_title))
            .bind(parse_i64(imdb_null(&row.is_adult)).unwrap_or(0))
            .bind(parse_i64(imdb_null(&row.start_year)))
            .bind(parse_i64(imdb_null(&row.end_year)))
            .bind(parse_i64(imdb_null(&row.runtime_minutes)))
            .bind(imdb_null(&row.genres))
            .execute(&mut *tx)
            .await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }

    async fn import_title_ratings(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<TitleRatingsRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO title_ratings (imdb_id, average_rating, num_votes)
                VALUES (?, ?, ?)
                ON CONFLICT(imdb_id) DO UPDATE SET
                    average_rating = excluded.average_rating,
                    num_votes = excluded.num_votes
                "#,
            )
            .bind(row.imdb_id)
            .bind(parse_f64(imdb_null(&row.average_rating)))
            .bind(parse_i64(imdb_null(&row.num_votes)))
            .execute(&mut *tx)
            .await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }

    async fn import_title_akas(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<TitleAkasRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO title_akas (imdb_id, ordering, title, region, language, types, attributes, is_original_title)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(imdb_id, ordering, title) DO UPDATE SET
                    region = excluded.region,
                    language = excluded.language,
                    types = excluded.types,
                    attributes = excluded.attributes,
                    is_original_title = excluded.is_original_title
                "#,
            )
            .bind(row.imdb_id)
            .bind(parse_i64(imdb_null(&row.ordering)))
            .bind(row.title)
            .bind(imdb_null(&row.region))
            .bind(imdb_null(&row.language))
            .bind(imdb_null(&row.types))
            .bind(imdb_null(&row.attributes))
            .bind(parse_i64(imdb_null(&row.is_original_title)))
            .execute(&mut *tx)
            .await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }

    async fn import_title_crew(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<TitleCrewRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO title_crew (imdb_id, directors, writers)
                VALUES (?, ?, ?)
                ON CONFLICT(imdb_id) DO UPDATE SET
                    directors = excluded.directors,
                    writers = excluded.writers
                "#,
            )
            .bind(&row.imdb_id)
            .bind(imdb_null(&row.directors))
            .bind(imdb_null(&row.writers))
            .execute(&mut *tx)
            .await?;

            insert_credit_list(&mut tx, &row.imdb_id, imdb_null(&row.directors), "director")
                .await?;
            insert_credit_list(&mut tx, &row.imdb_id, imdb_null(&row.writers), "writer").await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }

    async fn import_title_principals(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<TitlePrincipalsRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO credits (imdb_id, imdb_name_id, ordering, category, job, characters, source)
                VALUES (?, ?, ?, ?, ?, ?, 'imdb')
                ON CONFLICT(imdb_id, imdb_name_id, ordering, category, source) DO UPDATE SET
                    job = excluded.job,
                    characters = excluded.characters
                "#,
            )
            .bind(row.imdb_id)
            .bind(row.imdb_name_id)
            .bind(parse_i64(imdb_null(&row.ordering)))
            .bind(row.category)
            .bind(imdb_null(&row.job))
            .bind(imdb_null(&row.characters))
            .execute(&mut *tx)
            .await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }

    async fn import_title_episode(&self, path: &Path) -> Result<ImportStats> {
        let mut tx = self.db.pool().begin().await?;
        let mut stats = ImportStats::default();
        for row in read_gzip_tsv::<TitleEpisodeRow>(path)? {
            let row = row?;
            stats.rows_seen += 1;
            sqlx::query(
                r#"
                INSERT INTO episode_edges (imdb_id, parent_imdb_id, season_number, episode_number)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(imdb_id) DO UPDATE SET
                    parent_imdb_id = excluded.parent_imdb_id,
                    season_number = excluded.season_number,
                    episode_number = excluded.episode_number
                "#,
            )
            .bind(row.imdb_id)
            .bind(row.parent_imdb_id)
            .bind(parse_i64(imdb_null(&row.season_number)))
            .bind(parse_i64(imdb_null(&row.episode_number)))
            .execute(&mut *tx)
            .await?;
            stats.rows_inserted += 1;
        }
        tx.commit().await?;
        Ok(stats)
    }
}

async fn insert_credit_list(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    imdb_id: &str,
    ids: Option<&str>,
    category: &str,
) -> Result<()> {
    if let Some(ids) = ids {
        for (index, person_id) in ids.split(',').enumerate() {
            sqlx::query(
                r#"
                INSERT INTO credits (imdb_id, imdb_name_id, ordering, category, source)
                VALUES (?, ?, ?, ?, 'imdb')
                ON CONFLICT(imdb_id, imdb_name_id, ordering, category, source) DO NOTHING
                "#,
            )
            .bind(imdb_id)
            .bind(person_id)
            .bind(index as i64 + 1)
            .bind(category)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

fn parse_i64(value: Option<&str>) -> Option<i64> {
    value.and_then(|item| item.parse::<i64>().ok())
}

fn parse_f64(value: Option<&str>) -> Option<f64> {
    value.and_then(|item| item.parse::<f64>().ok())
}

fn dataset_priority(dataset_name: &str) -> usize {
    match dataset_name {
        "name.basics.tsv.gz" => 0,
        "title.basics.tsv.gz" => 1,
        "title.ratings.tsv.gz" => 2,
        "title.akas.tsv.gz" => 3,
        "title.crew.tsv.gz" => 4,
        "title.principals.tsv.gz" => 5,
        "title.episode.tsv.gz" => 6,
        _ => usize::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::{ImdbImporter, dataset_priority, parse_i64};
    use crate::IMPORTER_VERSION;
    use cinegraph_core::{
        AppConfig, AppPaths,
        config::{
            DataConfig, FetchConfig, GraphConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig,
            SqliteConfig, TmdbSourceConfig,
        },
    };
    use cinegraph_db::{Database, queries};
    use flate2::{Compression, write::GzEncoder};
    use std::{io::Write, path::Path};
    use tempfile::tempdir;

    #[test]
    fn parse_i64_handles_invalid_values() {
        assert_eq!(parse_i64(Some("42")), Some(42));
        assert_eq!(parse_i64(Some("bad")), None);
        assert_eq!(parse_i64(None), None);
    }

    #[test]
    fn dataset_priority_matches_import_dependencies() {
        assert!(
            dataset_priority("name.basics.tsv.gz") < dataset_priority("title.principals.tsv.gz")
        );
        assert!(dataset_priority("title.basics.tsv.gz") < dataset_priority("title.ratings.tsv.gz"));
        assert!(dataset_priority("title.basics.tsv.gz") < dataset_priority("title.crew.tsv.gz"));
    }

    #[tokio::test]
    async fn import_is_idempotent_and_lookupable() {
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
                    datasets: vec![
                        "name.basics.tsv.gz".to_string(),
                        "title.basics.tsv.gz".to_string(),
                    ],
                },
                tmdb: TmdbSourceConfig {
                    enabled: false,
                    export_base_url: "http://localhost/exports/".to_string(),
                    api_base_url: "http://localhost/api".to_string(),
                    api_read_access_token: String::new(),
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
        let db = Database::connect(&config, &paths).await.expect("connect");
        db.migrate().await.expect("migrate");

        let names_path = write_gzip_fixture(
            &paths.raw_source_dir("imdb").join("name.basics.tsv.gz"),
            "nconst\tprimaryName\tbirthYear\tdeathYear\tprimaryProfession\tknownForTitles\nnm0000001\tFred Astaire\t1899\t1987\tactor,soundtrack,producer\ttt0072308\n",
        );
        let titles_path = write_gzip_fixture(
            &paths.raw_source_dir("imdb").join("title.basics.tsv.gz"),
            "tconst\ttitleType\tprimaryTitle\toriginalTitle\tisAdult\tstartYear\tendYear\truntimeMinutes\tgenres\ntt0000001\tshort\tCarmencita\tCarmencita\t0\t1894\t\\N\t1\tDocumentary,Short\n",
        );

        let dataset_people = queries::upsert_dataset(
            db.pool(),
            "imdb",
            "name.basics.tsv.gz",
            "http://localhost/name.basics.tsv.gz",
        )
        .await
        .expect("dataset");
        let dataset_titles = queries::upsert_dataset(
            db.pool(),
            "imdb",
            "title.basics.tsv.gz",
            "http://localhost/title.basics.tsv.gz",
        )
        .await
        .expect("dataset");

        let artifact_people = queries::insert_artifact(
            db.pool(),
            dataset_people.id,
            "http://localhost/name.basics.tsv.gz",
            &names_path.to_string_lossy(),
            "hash-people",
            1,
            None,
            None,
        )
        .await
        .expect("artifact");
        let artifact_titles = queries::insert_artifact(
            db.pool(),
            dataset_titles.id,
            "http://localhost/title.basics.tsv.gz",
            &titles_path.to_string_lossy(),
            "hash-titles",
            1,
            None,
            None,
        )
        .await
        .expect("artifact");

        let importer = ImdbImporter::new(&db);
        let first_people = importer
            .import_artifact("name.basics.tsv.gz", &artifact_people)
            .await
            .expect("import names");
        let second_people = importer
            .import_artifact("name.basics.tsv.gz", &artifact_people)
            .await
            .expect("second import names");
        let first_titles = importer
            .import_artifact("title.basics.tsv.gz", &artifact_titles)
            .await
            .expect("import titles");

        assert_eq!(first_people.rows_inserted, 1);
        assert_eq!(second_people.rows_skipped, 1);
        assert_eq!(first_titles.rows_inserted, 1);

        let people = queries::lookup_person(db.pool(), "Fred")
            .await
            .expect("lookup people");
        let titles = queries::lookup_title(db.pool(), "Carmencita")
            .await
            .expect("lookup titles");
        assert_eq!(people.len(), 1);
        assert_eq!(titles.len(), 1);

        let import_runs: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM import_runs WHERE importer_version = ?")
                .bind(IMPORTER_VERSION)
                .fetch_one(db.pool())
                .await
                .expect("count");
        assert_eq!(import_runs.0, 2);
    }

    #[tokio::test]
    async fn import_latest_respects_foreign_key_order() {
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
                    datasets: vec![
                        "title.principals.tsv.gz".to_string(),
                        "title.basics.tsv.gz".to_string(),
                        "name.basics.tsv.gz".to_string(),
                    ],
                },
                tmdb: TmdbSourceConfig {
                    enabled: false,
                    export_base_url: "http://localhost/exports/".to_string(),
                    api_base_url: "http://localhost/api".to_string(),
                    api_read_access_token: String::new(),
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
        let db = Database::connect(&config, &paths).await.expect("connect");
        db.migrate().await.expect("migrate");

        let names_path = write_gzip_fixture(
            &paths.raw_source_dir("imdb").join("name.basics.tsv.gz"),
            "nconst\tprimaryName\tbirthYear\tdeathYear\tprimaryProfession\tknownForTitles\nnm0000001\tFred Astaire\t1899\t1987\tactor,soundtrack,producer\ttt0000001\n",
        );
        let titles_path = write_gzip_fixture(
            &paths.raw_source_dir("imdb").join("title.basics.tsv.gz"),
            "tconst\ttitleType\tprimaryTitle\toriginalTitle\tisAdult\tstartYear\tendYear\truntimeMinutes\tgenres\ntt0000001\tshort\tCarmencita\tCarmencita\t0\t1894\t\\N\t1\tDocumentary,Short\n",
        );
        let principals_path = write_gzip_fixture(
            &paths.raw_source_dir("imdb").join("title.principals.tsv.gz"),
            "tconst\tordering\tnconst\tcategory\tjob\tcharacters\ntt0000001\t1\tnm0000001\tactor\t\\N\t[\"Carmencita\"]\n",
        );

        for (dataset_name, path, hash) in [
            (
                "title.principals.tsv.gz",
                principals_path.as_path(),
                "hash-principals",
            ),
            ("title.basics.tsv.gz", titles_path.as_path(), "hash-titles"),
            ("name.basics.tsv.gz", names_path.as_path(), "hash-people"),
        ] {
            let dataset = queries::upsert_dataset(
                db.pool(),
                "imdb",
                dataset_name,
                &format!("http://localhost/{dataset_name}"),
            )
            .await
            .expect("dataset");
            queries::insert_artifact(
                db.pool(),
                dataset.id,
                &format!("http://localhost/{dataset_name}"),
                &path.to_string_lossy(),
                hash,
                1,
                None,
                None,
            )
            .await
            .expect("artifact");
        }

        let importer = ImdbImporter::new(&db);
        let results = importer.import_latest().await.expect("import latest");

        assert_eq!(
            results
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "name.basics.tsv.gz",
                "title.basics.tsv.gz",
                "title.principals.tsv.gz",
            ]
        );

        let credits_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM credits")
            .fetch_one(db.pool())
            .await
            .expect("count credits");
        assert_eq!(credits_count.0, 1);
    }

    fn write_gzip_fixture(path: &Path, body: &str) -> std::path::PathBuf {
        let file = std::fs::File::create(path).expect("fixture file");
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder
            .write_all(body.as_bytes())
            .expect("write fixture body");
        encoder.finish().expect("finish gzip");
        path.to_path_buf()
    }
}
