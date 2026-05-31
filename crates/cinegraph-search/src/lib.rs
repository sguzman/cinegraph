use cinegraph_core::{AppPaths, CinegraphError, Result};
use cinegraph_db::{
    Database,
    models::{LookupPerson, LookupTitle},
    queries,
};
use serde::Serialize;
use tantivy::{
    Index, ReloadPolicy, TantivyDocument,
    collector::TopDocs,
    query::QueryParser,
    schema::{Field, STORED, STRING, Schema, TEXT, Value},
};
use tracing::info;

#[derive(Debug, Clone, Serialize)]
pub struct SearchBuildStats {
    pub indexed_titles: usize,
    pub indexed_people: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit<T> {
    pub score: f32,
    pub record: T,
}

pub struct SearchService {
    title_index: Index,
    title_schema: SearchSchema,
    person_index: Index,
    person_schema: SearchSchema,
}

#[derive(Clone)]
struct SearchSchema {
    id: Field,
    name: Field,
    alt_name: Field,
    context: Field,
}

impl SearchService {
    pub fn open(paths: &AppPaths) -> Result<Self> {
        let title_index = open_or_create_index(paths.title_search_index_dir())?;
        let person_index = open_or_create_index(paths.person_search_index_dir())?;

        Ok(Self {
            title_schema: search_schema(&title_index.schema()),
            person_schema: search_schema(&person_index.schema()),
            title_index,
            person_index,
        })
    }

    pub async fn rebuild(&self, db: &Database) -> Result<SearchBuildStats> {
        let titles = queries::titles_for_search_index(db.pool()).await?;
        let people = queries::people_for_search_index(db.pool()).await?;

        rebuild_title_index(&self.title_index, &self.title_schema, &titles)?;
        rebuild_person_index(&self.person_index, &self.person_schema, &people)?;

        info!(
            indexed_titles = titles.len(),
            indexed_people = people.len(),
            "tantivy index rebuilt"
        );

        Ok(SearchBuildStats {
            indexed_titles: titles.len(),
            indexed_people: people.len(),
        })
    }

    pub async fn search_titles(
        &self,
        db: &Database,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit<LookupTitle>>> {
        let hits = self.search_ids(&self.title_index, &self.title_schema, query, limit)?;
        let ids = hits.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>();
        let records = queries::titles_by_ids_in_order(db.pool(), &ids).await?;
        let by_id = records
            .into_iter()
            .map(|record| (record.imdb_id.clone(), record))
            .collect::<std::collections::HashMap<_, _>>();

        Ok(hits
            .into_iter()
            .filter_map(|(score, id)| by_id.get(&id).cloned().map(|record| SearchHit { score, record }))
            .collect())
    }

    pub async fn search_people(
        &self,
        db: &Database,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit<LookupPerson>>> {
        let hits = self.search_ids(&self.person_index, &self.person_schema, query, limit)?;
        let ids = hits.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>();
        let records = queries::people_by_ids_in_order(db.pool(), &ids).await?;
        let by_id = records
            .into_iter()
            .map(|record| (record.imdb_name_id.clone(), record))
            .collect::<std::collections::HashMap<_, _>>();

        Ok(hits
            .into_iter()
            .filter_map(|(score, id)| by_id.get(&id).cloned().map(|record| SearchHit { score, record }))
            .collect())
    }

    fn search_ids(
        &self,
        index: &Index,
        schema: &SearchSchema,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(f32, String)>> {
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(tantivy_error)?;
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(index, vec![schema.name, schema.alt_name, schema.context]);
        let parsed = parser.parse_query(query).map_err(tantivy_error)?;
        let docs = searcher
            .search(&parsed, &TopDocs::with_limit(limit))
            .map_err(tantivy_error)?;

        let mut results = Vec::with_capacity(docs.len());
        for (score, doc_address) in docs {
            let doc: TantivyDocument = searcher.doc(doc_address).map_err(tantivy_error)?;
            if let Some(id) = doc
                .get_first(schema.id)
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
            {
                results.push((score, id));
            }
        }
        Ok(results)
    }
}

fn open_or_create_index(path: std::path::PathBuf) -> Result<Index> {
    std::fs::create_dir_all(&path)?;
    match Index::open_in_dir(&path) {
        Ok(index) => Ok(index),
        Err(_) => Index::create_in_dir(&path, make_schema()).map_err(tantivy_error),
    }
}

fn make_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("id", STRING | STORED);
    builder.add_text_field("name", TEXT | STORED);
    builder.add_text_field("alt_name", TEXT);
    builder.add_text_field("context", TEXT);
    builder.build()
}

fn search_schema(schema: &Schema) -> SearchSchema {
    SearchSchema {
        id: schema.get_field("id").expect("id field"),
        name: schema.get_field("name").expect("name field"),
        alt_name: schema.get_field("alt_name").expect("alt_name field"),
        context: schema.get_field("context").expect("context field"),
    }
}

fn rebuild_title_index(
    index: &Index,
    schema: &SearchSchema,
    titles: &[cinegraph_db::models::IndexableTitle],
) -> Result<()> {
    let mut writer = index.writer(50_000_000).map_err(tantivy_error)?;
    writer.delete_all_documents().map_err(tantivy_error)?;
    for title in titles {
        let mut doc = TantivyDocument::default();
        doc.add_text(schema.id, &title.imdb_id);
        doc.add_text(schema.name, &title.primary_title);
        if let Some(original_title) = &title.original_title {
            doc.add_text(schema.alt_name, original_title);
        }
        let context = [
            Some(title.title_type.as_str()),
            title.genres.as_deref(),
            title.people_text.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        if !context.is_empty() {
            doc.add_text(schema.context, context);
        }
        writer.add_document(doc).map_err(tantivy_error)?;
    }
    writer.commit().map_err(tantivy_error)?;
    Ok(())
}

fn rebuild_person_index(
    index: &Index,
    schema: &SearchSchema,
    people: &[cinegraph_db::models::IndexablePerson],
) -> Result<()> {
    let mut writer = index.writer(50_000_000).map_err(tantivy_error)?;
    writer.delete_all_documents().map_err(tantivy_error)?;
    for person in people {
        let mut doc = TantivyDocument::default();
        doc.add_text(schema.id, &person.imdb_name_id);
        doc.add_text(schema.name, &person.primary_name);
        if let Some(primary_professions) = &person.primary_professions {
            doc.add_text(schema.alt_name, primary_professions);
        }
        let context = [
            person.primary_professions.as_deref(),
            person.title_text.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        if !context.is_empty() {
            doc.add_text(schema.context, context);
        }
        writer.add_document(doc).map_err(tantivy_error)?;
    }
    writer.commit().map_err(tantivy_error)?;
    Ok(())
}

fn tantivy_error(error: impl std::fmt::Display) -> CinegraphError {
    CinegraphError::Other(format!("tantivy error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinegraph_core::{
        AppConfig,
        config::{
            DataConfig, FetchConfig, GraphConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig,
            SqliteConfig, TmdbSourceConfig,
        },
    };
    use tempfile::tempdir;

    #[test]
    fn search_index_rebuilds_and_hydrates_results() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
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
            let db = Database::connect(&config, &paths).await.expect("db");
            db.migrate().await.expect("migrate");

            sqlx::query("INSERT INTO titles (imdb_id, title_type, primary_title, original_title, is_adult, start_year, genres) VALUES ('tt1', 'movie', 'Seven Samurai', 'Shichinin no samurai', 0, 1954, 'Drama'), ('tt2', 'movie', 'Ikiru', 'Ikiru', 0, 1952, 'Drama')")
                .execute(db.pool())
                .await
                .expect("titles");
            sqlx::query("INSERT INTO people (imdb_name_id, primary_name, birth_year, primary_professions) VALUES ('nm1', 'Akira Kurosawa', 1910, 'director,writer'), ('nm2', 'Takashi Shimura', 1905, 'actor')")
                .execute(db.pool())
                .await
                .expect("people");
            sqlx::query("INSERT INTO credits (imdb_id, imdb_name_id, ordering, category, source) VALUES ('tt1', 'nm1', 1, 'director', 'imdb'), ('tt1', 'nm2', 2, 'actor', 'imdb'), ('tt2', 'nm1', 1, 'director', 'imdb'), ('tt2', 'nm2', 2, 'actor', 'imdb')")
                .execute(db.pool())
                .await
                .expect("credits");

            let service = SearchService::open(&paths).expect("service");
            let stats = service.rebuild(&db).await.expect("rebuild");
            assert_eq!(stats.indexed_titles, 2);
            assert_eq!(stats.indexed_people, 2);

            let title_hits = service
                .search_titles(&db, "samurai kurosawa", 5)
                .await
                .expect("title search");
            assert_eq!(title_hits.len(), 1);
            assert_eq!(title_hits[0].record.imdb_id, "tt1");

            let person_hits = service
                .search_people(&db, "kurosawa ikiru", 5)
                .await
                .expect("person search");
            assert_eq!(person_hits.len(), 1);
            assert_eq!(person_hits[0].record.imdb_name_id, "nm1");
        });
    }
}
