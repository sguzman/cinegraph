use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

use cinegraph_core::{AppConfig, CinegraphError, Result};
use cinegraph_db::{Database, models::DownloadArtifact, queries};
use flate2::read::MultiGzDecoder;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::info;

pub const IMPORTER_NAME: &str = "wikidata-dump";
pub const IMPORTER_VERSION: &str = "0.1.0";

const SELECTED_PROPERTIES: &[&str] = &[
    "P31", "P57", "P161", "P136", "P364", "P495", "P577", "P569", "P570",
];

#[derive(Debug, Clone, Default, Serialize)]
pub struct WikidataImportStats {
    pub entities_seen: i64,
    pub entities_imported: i64,
    pub title_links_created: i64,
    pub person_links_created: i64,
    pub claims_imported: i64,
    pub imports_skipped: i64,
}

pub struct WikidataImporter<'a> {
    db: &'a Database,
}

impl<'a> WikidataImporter<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub async fn import_dump(&self, config: &AppConfig) -> Result<WikidataImportStats> {
        let dump_path = PathBuf::from(&config.sources.wikidata.dump_path);
        if config.sources.wikidata.dump_path.is_empty() {
            return Err(CinegraphError::Config(
                "sources.wikidata.dump_path must be set to import Wikidata".to_string(),
            ));
        }
        if !dump_path.exists() {
            return Err(CinegraphError::Config(format!(
                "configured Wikidata dump does not exist: {}",
                dump_path.display()
            )));
        }

        let artifact = self.register_dump_artifact(&dump_path).await?;
        let started = queries::try_begin_import_run(
            self.db.pool(),
            artifact.id,
            IMPORTER_NAME,
            IMPORTER_VERSION,
        )
        .await?;

        if !started {
            return Ok(WikidataImportStats {
                imports_skipped: 1,
                ..WikidataImportStats::default()
            });
        }

        let result = self
            .import_rows(&dump_path, &config.sources.wikidata.language)
            .await;
        match result {
            Ok(stats) => {
                queries::finish_import_run(
                    self.db.pool(),
                    artifact.id,
                    IMPORTER_NAME,
                    IMPORTER_VERSION,
                    "completed",
                    stats.entities_seen,
                    stats.entities_imported
                        + stats.title_links_created
                        + stats.person_links_created,
                    0,
                    stats.imports_skipped,
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

    async fn register_dump_artifact(&self, dump_path: &Path) -> Result<DownloadArtifact> {
        let canonical = dump_path.canonicalize()?;
        let sha256 = sha256_file(&canonical)?;
        let byte_len = canonical.metadata()?.len() as i64;
        let canonical_url = format!("file://{}", canonical.display());
        let dataset =
            queries::upsert_dataset(self.db.pool(), "wikidata", "wikidata-dump", &canonical_url)
                .await?;

        if let Some(existing) =
            queries::artifact_by_hash(self.db.pool(), dataset.id, &sha256).await?
        {
            return Ok(existing);
        }

        queries::insert_artifact(
            self.db.pool(),
            dataset.id,
            &canonical_url,
            &canonical.to_string_lossy(),
            &sha256,
            byte_len,
            None,
            None,
        )
        .await
    }

    async fn import_rows(&self, dump_path: &Path, language: &str) -> Result<WikidataImportStats> {
        queries::clear_wikidata_import(self.db.pool()).await?;

        let mut stats = WikidataImportStats::default();
        let reader = open_dump_reader(dump_path)?;

        for line in reader.lines() {
            let raw = line?;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed == "[" || trimmed == "]" {
                continue;
            }

            let normalized = trimmed.trim_end_matches(',');
            let entity: Value = serde_json::from_str(normalized)?;
            if entity.get("type").and_then(Value::as_str) != Some("item") {
                continue;
            }

            let Some(wikidata_id) = entity.get("id").and_then(Value::as_str) else {
                continue;
            };

            stats.entities_seen += 1;

            let label = localized_text(entity.get("labels"), language);
            let description = localized_text(entity.get("descriptions"), language);
            queries::upsert_wikidata_entity(
                self.db.pool(),
                wikidata_id,
                label.as_deref(),
                description.as_deref(),
                Some("item"),
            )
            .await?;
            stats.entities_imported += 1;

            let imdb_ids = extract_imdb_ids(&entity);
            if imdb_ids.is_empty() {
                continue;
            }

            let mut linked = false;
            for imdb_id in imdb_ids {
                if imdb_id.starts_with("tt")
                    && queries::title_exists(self.db.pool(), &imdb_id).await?
                {
                    if queries::link_title_to_wikidata_entity(self.db.pool(), &imdb_id, wikidata_id)
                        .await?
                    {
                        stats.title_links_created += 1;
                    }
                    linked = true;
                } else if imdb_id.starts_with("nm")
                    && queries::person_exists(self.db.pool(), &imdb_id).await?
                {
                    if queries::link_person_to_wikidata_entity(
                        self.db.pool(),
                        &imdb_id,
                        wikidata_id,
                    )
                    .await?
                    {
                        stats.person_links_created += 1;
                    }
                    linked = true;
                }
            }

            if linked {
                stats.claims_imported += self.import_claims(wikidata_id, &entity).await?;
            }
        }

        info!(
            entities_seen = stats.entities_seen,
            entities_imported = stats.entities_imported,
            title_links_created = stats.title_links_created,
            person_links_created = stats.person_links_created,
            claims_imported = stats.claims_imported,
            "Wikidata dump imported"
        );

        Ok(stats)
    }

    async fn import_claims(&self, wikidata_id: &str, entity: &Value) -> Result<i64> {
        let Some(claims) = entity.get("claims").and_then(Value::as_object) else {
            return Ok(0);
        };

        let mut inserted = 0_i64;
        for property_id in SELECTED_PROPERTIES {
            let Some(statements) = claims.get(*property_id).and_then(Value::as_array) else {
                continue;
            };
            for (ordinal, statement) in statements.iter().enumerate() {
                let Some(mainsnak) = statement.get("mainsnak") else {
                    continue;
                };
                let datatype = mainsnak.get("datatype").and_then(Value::as_str);
                let rank_name = statement.get("rank").and_then(Value::as_str);
                let Some(parsed) = parse_claim_value(mainsnak) else {
                    continue;
                };

                if let Some(value_wikidata_id) = parsed.value_wikidata_id.as_deref() {
                    queries::upsert_wikidata_entity(
                        self.db.pool(),
                        value_wikidata_id,
                        None,
                        None,
                        Some("item"),
                    )
                    .await?;
                }

                queries::insert_wikidata_claim(
                    self.db.pool(),
                    wikidata_id,
                    property_id,
                    parsed.value_type,
                    parsed.value_text.as_deref(),
                    parsed.value_wikidata_id.as_deref(),
                    datatype,
                    rank_name,
                    ordinal as i64,
                    Some(&statement.to_string()),
                )
                .await?;
                inserted += 1;
            }
        }

        Ok(inserted)
    }
}

struct ParsedClaimValue {
    value_type: &'static str,
    value_text: Option<String>,
    value_wikidata_id: Option<String>,
}

fn open_dump_reader(path: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(path)?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("gz") {
        Ok(Box::new(BufReader::new(MultiGzDecoder::new(file))))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn localized_text(node: Option<&Value>, language: &str) -> Option<String> {
    let map = node?.as_object()?;
    map.get(language)
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            map.get("en")
                .and_then(|value| value.get("value"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn extract_imdb_ids(entity: &Value) -> Vec<String> {
    entity
        .get("claims")
        .and_then(Value::as_object)
        .and_then(|claims| claims.get("P345"))
        .and_then(Value::as_array)
        .map(|statements| {
            statements
                .iter()
                .filter_map(|statement| {
                    let mainsnak = statement.get("mainsnak")?;
                    let datavalue = mainsnak.get("datavalue")?;
                    match datavalue.get("value") {
                        Some(Value::String(value)) => Some(value.clone()),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_claim_value(mainsnak: &Value) -> Option<ParsedClaimValue> {
    let datavalue = mainsnak.get("datavalue")?;
    let value = datavalue.get("value")?;

    match value {
        Value::String(text) => Some(ParsedClaimValue {
            value_type: "string",
            value_text: Some(text.clone()),
            value_wikidata_id: None,
        }),
        Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(Value::as_str) {
                return Some(ParsedClaimValue {
                    value_type: "wikidata_item",
                    value_text: None,
                    value_wikidata_id: Some(id.to_string()),
                });
            }
            if let Some(time) = object.get("time").and_then(Value::as_str) {
                return Some(ParsedClaimValue {
                    value_type: "time",
                    value_text: Some(time.to_string()),
                    value_wikidata_id: None,
                });
            }
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                return Some(ParsedClaimValue {
                    value_type: "monolingual_text",
                    value_text: Some(text.to_string()),
                    value_wikidata_id: None,
                });
            }
            if let Some(amount) = object.get("amount").and_then(Value::as_str) {
                return Some(ParsedClaimValue {
                    value_type: "quantity",
                    value_text: Some(amount.to_string()),
                    value_wikidata_id: None,
                });
            }
            None
        }
        Value::Number(number) => Some(ParsedClaimValue {
            value_type: "number",
            value_text: Some(number.to_string()),
            value_wikidata_id: None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinegraph_core::{
        AppConfig, AppPaths,
        config::{
            DataConfig, FetchConfig, GraphConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig,
            SqliteConfig, TmdbSourceConfig, WikidataSourceConfig,
        },
    };
    use cinegraph_db::Database;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn imports_local_wikidata_dump_and_skips_repeat_run() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join(".data");
        let dump_path = temp.path().join("wikidata-mini.json.gz");
        write_fixture_dump(&dump_path);

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
                    datasets: vec!["title.basics.tsv.gz".to_string()],
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
                wikidata: WikidataSourceConfig {
                    enabled: true,
                    dump_path: dump_path.to_string_lossy().to_string(),
                    language: "en".to_string(),
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
            .expect("title");
        sqlx::query("INSERT INTO people (imdb_name_id, primary_name) VALUES ('nm0000041', 'Akira Kurosawa')")
            .execute(db.pool())
            .await
            .expect("person");

        let importer = WikidataImporter::new(&db);
        let stats = importer.import_dump(&config).await.expect("import");
        assert_eq!(stats.title_links_created, 1);
        assert_eq!(stats.person_links_created, 1);
        assert!(stats.claims_imported >= 3);

        let counts = queries::stats(db.pool()).await.expect("stats");
        assert_eq!(counts["wikidata_entities"], 4);
        assert_eq!(counts["title_wikidata_links"], 1);
        assert_eq!(counts["person_wikidata_links"], 1);

        let second = importer.import_dump(&config).await.expect("second import");
        assert_eq!(second.imports_skipped, 1);
    }

    fn write_fixture_dump(path: &Path) {
        let file = File::create(path).expect("dump file");
        let mut encoder = GzEncoder::new(file, Compression::default());
        let lines = [
            r#"["#,
            r#"{"id":"Q745","type":"item","labels":{"en":{"language":"en","value":"Seven Samurai"}},"descriptions":{"en":{"language":"en","value":"1954 Japanese film"}},"claims":{"P345":[{"mainsnak":{"datatype":"external-id","datavalue":{"type":"string","value":"tt0047478"}}}],"P136":[{"mainsnak":{"datatype":"wikibase-item","datavalue":{"type":"wikibase-entityid","value":{"entity-type":"item","id":"Q1137136","numeric-id":1137136}}}}],"P577":[{"mainsnak":{"datatype":"time","datavalue":{"type":"time","value":{"time":"+1954-04-26T00:00:00Z"}}}}]}},"#,
            r#"{"id":"Q1137136","type":"item","labels":{"en":{"language":"en","value":"jidaigeki"}},"descriptions":{"en":{"language":"en","value":"Japanese period drama genre"}}},"#,
            r#"{"id":"Q3460","type":"item","labels":{"en":{"language":"en","value":"Akira Kurosawa"}},"claims":{"P345":[{"mainsnak":{"datatype":"external-id","datavalue":{"type":"string","value":"nm0000041"}}}],"P569":[{"mainsnak":{"datatype":"time","datavalue":{"type":"time","value":{"time":"+1910-03-23T00:00:00Z"}}}}]}},"#,
            r#"{"id":"Q17","type":"item","labels":{"en":{"language":"en","value":"Japan"}}}"#,
            r#"]"#,
        ];

        for line in lines {
            writeln!(encoder, "{line}").expect("write dump line");
        }
        encoder.finish().expect("finish dump");
    }
}
