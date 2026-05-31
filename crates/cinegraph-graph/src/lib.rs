use std::{collections::HashMap, fs, path::Path};

use cinegraph_core::{AppConfig, AppPaths, CinegraphError, Result};
use cinegraph_db::{Database, queries};
use oxigraph::{
    model::{GraphNameRef, Literal, NamedNode, Quad, Subject, Term},
    sparql::QueryResults,
    store::Store,
};
use serde::Serialize;
use tracing::info;

const SCHEMA_NAME: &str = "https://schema.org/name";
const SCHEMA_PERSON: &str = "https://schema.org/Person";
const SCHEMA_MOVIE: &str = "https://schema.org/Movie";
const SCHEMA_DIRECTOR: &str = "https://schema.org/director";
const SCHEMA_ACTOR: &str = "https://schema.org/actor";
const SCHEMA_DATE_PUBLISHED: &str = "https://schema.org/datePublished";

#[derive(Debug, Clone, Serialize)]
pub struct GraphBuildStats {
    pub titles_projected: usize,
    pub people_projected: usize,
    pub credits_projected: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNeighbor {
    pub direction: String,
    pub predicate: String,
    pub entity_id: String,
    pub entity_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollaborationHit {
    pub person_id: String,
    pub person_name: String,
    pub shared_titles: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphQueryOutput {
    Solutions {
        variables: Vec<String>,
        rows: Vec<HashMap<String, String>>,
    },
    Graph {
        triples: Vec<HashMap<String, String>>,
    },
    Boolean {
        value: bool,
    },
}

pub struct GraphService {
    store: Store,
    base_iri: String,
}

impl GraphService {
    pub fn open(config: &AppConfig, paths: &AppPaths) -> Result<Self> {
        fs::create_dir_all(paths.graph_store_dir())?;
        let store = Store::open(paths.graph_store_dir()).map_err(graph_error)?;
        Ok(Self {
            store,
            base_iri: config.graph.base_iri.clone(),
        })
    }

    pub async fn rebuild(&self, db: &Database) -> Result<GraphBuildStats> {
        self.store.clear().map_err(graph_error)?;

        let title_rows = queries::titles_for_graph(db.pool()).await?;
        let person_rows = queries::people_for_graph(db.pool()).await?;
        let credit_rows = queries::credits_for_graph(db.pool()).await?;

        for title in &title_rows {
            let title_node = self.title_node(&title.imdb_id)?;
            self.insert_quad(
                title_node.clone().into(),
                named_node("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")?,
                named_node(SCHEMA_MOVIE)?.into(),
            )?;
            self.insert_quad(
                title_node.clone().into(),
                named_node(SCHEMA_NAME)?,
                Literal::new_simple_literal(&title.primary_title).into(),
            )?;
            if let Some(year) = title.start_year {
                self.insert_quad(
                    title_node.clone().into(),
                    named_node(SCHEMA_DATE_PUBLISHED)?,
                    Literal::from(year).into(),
                )?;
            }
            if let Some(original_title) = &title.original_title {
                self.insert_quad(
                    title_node.into(),
                    self.cine_predicate("originalTitle")?,
                    Literal::new_simple_literal(original_title).into(),
                )?;
            }
        }

        for person in &person_rows {
            let person_node = self.person_node(&person.imdb_name_id)?;
            self.insert_quad(
                person_node.clone().into(),
                named_node("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")?,
                named_node(SCHEMA_PERSON)?.into(),
            )?;
            self.insert_quad(
                person_node.clone().into(),
                named_node(SCHEMA_NAME)?,
                Literal::new_simple_literal(&person.primary_name).into(),
            )?;
            if let Some(year) = person.birth_year {
                self.insert_quad(
                    person_node.clone().into(),
                    self.cine_predicate("birthYear")?,
                    Literal::from(year).into(),
                )?;
            }
            if let Some(year) = person.death_year {
                self.insert_quad(
                    person_node.into(),
                    self.cine_predicate("deathYear")?,
                    Literal::from(year).into(),
                )?;
            }
        }

        for credit in &credit_rows {
            let title_node = self.title_node(&credit.imdb_id)?;
            let person_node = self.person_node(&credit.imdb_name_id)?;
            let credit_node = self.credit_node(
                &credit.imdb_id,
                &credit.imdb_name_id,
                credit.ordering.unwrap_or_default(),
                credit.category.as_deref().unwrap_or("unknown"),
            )?;

            self.insert_quad(
                title_node.clone().into(),
                self.cine_predicate("hasParticipant")?,
                person_node.clone().into(),
            )?;
            self.insert_quad(
                person_node.clone().into(),
                self.cine_predicate("creditedOn")?,
                title_node.clone().into(),
            )?;
            self.insert_quad(
                title_node.clone().into(),
                self.cine_predicate("hasCredit")?,
                credit_node.clone().into(),
            )?;
            self.insert_quad(
                credit_node.clone().into(),
                self.cine_predicate("forTitle")?,
                title_node.clone().into(),
            )?;
            self.insert_quad(
                credit_node.clone().into(),
                self.cine_predicate("person")?,
                person_node.clone().into(),
            )?;

            if let Some(ordering) = credit.ordering {
                self.insert_quad(
                    credit_node.clone().into(),
                    self.cine_predicate("billingOrder")?,
                    Literal::from(ordering).into(),
                )?;
            }
            if let Some(category) = &credit.category {
                self.insert_quad(
                    credit_node.clone().into(),
                    self.cine_predicate("category")?,
                    Literal::new_simple_literal(category).into(),
                )?;
                match category.as_str() {
                    "director" => {
                        self.insert_quad(
                            title_node.clone().into(),
                            named_node(SCHEMA_DIRECTOR)?,
                            person_node.clone().into(),
                        )?;
                        self.insert_quad(
                            person_node.clone().into(),
                            self.cine_predicate("directed")?,
                            title_node.clone().into(),
                        )?;
                    }
                    "actor" | "actress" => {
                        self.insert_quad(
                            title_node.clone().into(),
                            named_node(SCHEMA_ACTOR)?,
                            person_node.clone().into(),
                        )?;
                        self.insert_quad(
                            person_node.clone().into(),
                            self.cine_predicate("actedIn")?,
                            title_node.clone().into(),
                        )?;
                    }
                    "writer" => {
                        self.insert_quad(
                            title_node.clone().into(),
                            self.cine_predicate("writer")?,
                            person_node.clone().into(),
                        )?;
                        self.insert_quad(
                            person_node.clone().into(),
                            self.cine_predicate("wrote")?,
                            title_node.clone().into(),
                        )?;
                    }
                    "producer" => {
                        self.insert_quad(
                            title_node.clone().into(),
                            self.cine_predicate("producer")?,
                            person_node.clone().into(),
                        )?;
                        self.insert_quad(
                            person_node.clone().into(),
                            self.cine_predicate("produced")?,
                            title_node.clone().into(),
                        )?;
                    }
                    _ => {}
                }
            }
            if let Some(job) = &credit.job {
                self.insert_quad(
                    credit_node.clone().into(),
                    self.cine_predicate("job")?,
                    Literal::new_simple_literal(job).into(),
                )?;
            }
            if let Some(characters) = &credit.characters {
                self.insert_quad(
                    credit_node.into(),
                    self.cine_predicate("characters")?,
                    Literal::new_simple_literal(characters).into(),
                )?;
            }
        }

        info!(
            titles_projected = title_rows.len(),
            people_projected = person_rows.len(),
            credits_projected = credit_rows.len(),
            "oxigraph graph rebuilt"
        );

        Ok(GraphBuildStats {
            titles_projected: title_rows.len(),
            people_projected: person_rows.len(),
            credits_projected: credit_rows.len(),
        })
    }

    pub fn query_file(&self, path: &Path) -> Result<GraphQueryOutput> {
        let query = fs::read_to_string(path)?;
        self.query(&query)
    }

    pub fn query(&self, query: &str) -> Result<GraphQueryOutput> {
        let results = self.store.query(query).map_err(graph_error)?;
        graph_query_output(results)
    }

    pub fn neighbors(&self, entity_id: &str) -> Result<Vec<GraphNeighbor>> {
        let entity_iri = self.entity_iri(entity_id);
        let query = format!(
            "PREFIX schema: <https://schema.org/>
             SELECT ?direction ?predicate ?neighbor ?name WHERE {{
               {{
                 BIND(\"out\" AS ?direction)
                 <{entity_iri}> ?predicate ?neighbor .
                 ?neighbor schema:name ?name .
                 FILTER(STRSTARTS(STR(?neighbor), \"{base}title/\") || STRSTARTS(STR(?neighbor), \"{base}person/\"))
               }}
               UNION
               {{
                 BIND(\"in\" AS ?direction)
                 ?neighbor ?predicate <{entity_iri}> .
                 ?neighbor schema:name ?name .
                 FILTER(STRSTARTS(STR(?neighbor), \"{base}title/\") || STRSTARTS(STR(?neighbor), \"{base}person/\"))
               }}
             }}
             ORDER BY ?direction ?predicate ?name",
            base = self.base_iri
        );

        let output = self.query(&query)?;
        let GraphQueryOutput::Solutions { rows, .. } = output else {
            return Err(CinegraphError::Graph(
                "neighbors query did not return solutions".to_string(),
            ));
        };

        let mut neighbors = Vec::with_capacity(rows.len());
        for row in rows {
            if let (Some(direction), Some(predicate), Some(neighbor), Some(name)) = (
                row.get("direction"),
                row.get("predicate"),
                row.get("neighbor"),
                row.get("name"),
            ) {
                neighbors.push(GraphNeighbor {
                    direction: direction.clone(),
                    predicate: compact_iri(predicate),
                    entity_id: local_entity_id(neighbor),
                    entity_name: name.clone(),
                });
            }
        }
        Ok(neighbors)
    }

    pub fn collaborations(&self, person_id: &str) -> Result<Vec<CollaborationHit>> {
        let person_iri = self.entity_iri(person_id);
        let query = format!(
            "PREFIX schema: <https://schema.org/>
             PREFIX cine: <{base}>
             SELECT ?other ?name (COUNT(DISTINCT ?title) AS ?shared) WHERE {{
               <{person_iri}> cine:creditedOn ?title .
               ?other cine:creditedOn ?title .
               ?other schema:name ?name .
               FILTER(?other != <{person_iri}>)
               FILTER(STRSTARTS(STR(?other), \"{base}person/\"))
             }}
             GROUP BY ?other ?name
             ORDER BY DESC(?shared) ?name",
            base = self.base_iri
        );

        let output = self.query(&query)?;
        let GraphQueryOutput::Solutions { rows, .. } = output else {
            return Err(CinegraphError::Graph(
                "collaborations query did not return solutions".to_string(),
            ));
        };

        let mut hits = Vec::with_capacity(rows.len());
        for row in rows {
            if let (Some(other), Some(name), Some(shared)) =
                (row.get("other"), row.get("name"), row.get("shared"))
            {
                hits.push(CollaborationHit {
                    person_id: local_entity_id(other),
                    person_name: name.clone(),
                    shared_titles: shared.parse::<usize>().unwrap_or_default(),
                });
            }
        }
        Ok(hits)
    }

    fn title_node(&self, imdb_id: &str) -> Result<NamedNode> {
        named_node(&format!("{}title/{imdb_id}", self.base_iri))
    }

    fn person_node(&self, imdb_name_id: &str) -> Result<NamedNode> {
        named_node(&format!("{}person/{imdb_name_id}", self.base_iri))
    }

    fn credit_node(
        &self,
        imdb_id: &str,
        imdb_name_id: &str,
        ordering: i64,
        category: &str,
    ) -> Result<NamedNode> {
        named_node(&format!(
            "{}credit/{imdb_id}/{imdb_name_id}/{ordering}/{}",
            self.base_iri,
            sanitize(category)
        ))
    }

    fn cine_predicate(&self, suffix: &str) -> Result<NamedNode> {
        named_node(&format!("{}{}", self.base_iri, suffix))
    }

    fn entity_iri(&self, entity_id: &str) -> String {
        if entity_id.starts_with("tt") {
            format!("{}title/{entity_id}", self.base_iri)
        } else {
            format!("{}person/{entity_id}", self.base_iri)
        }
    }

    fn insert_quad(&self, subject: Subject, predicate: NamedNode, object: Term) -> Result<()> {
        self.store
            .insert(&Quad::new(subject, predicate, object, GraphNameRef::DefaultGraph))
            .map_err(graph_error)?;
        Ok(())
    }
}

fn graph_query_output(results: QueryResults) -> Result<GraphQueryOutput> {
    match results {
        QueryResults::Solutions(solutions) => {
            let variables = solutions
                .variables()
                .iter()
                .map(|var| var.as_str().to_string())
                .collect::<Vec<_>>();
            let mut rows = Vec::new();
            for solution in solutions {
                let solution = solution.map_err(graph_error)?;
                let mut row = HashMap::new();
                for (variable, term) in solution.iter() {
                    row.insert(variable.as_str().to_string(), term_value(term));
                }
                rows.push(row);
            }
            Ok(GraphQueryOutput::Solutions { variables, rows })
        }
        QueryResults::Graph(triples) => {
            let mut rows = Vec::new();
            for triple in triples {
                let triple = triple.map_err(graph_error)?;
                rows.push(HashMap::from([
                    (
                        "subject".to_string(),
                        match triple.subject {
                            Subject::NamedNode(node) => node.as_str().to_string(),
                            _ => triple.subject.to_string(),
                        },
                    ),
                    ("predicate".to_string(), triple.predicate.as_str().to_string()),
                    ("object".to_string(), term_value(&triple.object)),
                ]));
            }
            Ok(GraphQueryOutput::Graph { triples: rows })
        }
        QueryResults::Boolean(value) => Ok(GraphQueryOutput::Boolean { value }),
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || char == '-' || char == '_' {
                char
            } else {
                '_'
            }
        })
        .collect()
}

fn local_entity_id(iri: &str) -> String {
    iri.rsplit('/')
        .next()
        .unwrap_or(iri)
        .trim_matches('"')
        .trim_matches('<')
        .trim_matches('>')
        .to_string()
}

fn compact_iri(iri: &str) -> String {
    iri.trim_matches('"')
        .trim_matches('<')
        .trim_matches('>')
        .rsplit('/')
        .next()
        .unwrap_or(iri)
        .to_string()
}

fn graph_error(error: impl std::fmt::Display) -> CinegraphError {
    CinegraphError::Graph(error.to_string())
}

fn named_node(value: &str) -> Result<NamedNode> {
    NamedNode::new(value).map_err(graph_error)
}

fn term_value(term: &Term) -> String {
    match term {
        Term::NamedNode(node) => node.as_str().to_string(),
        Term::Literal(literal) => literal.value().to_string(),
        _ => term.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinegraph_core::{
        AppConfig, AppPaths,
        config::{
            DataConfig, FetchConfig, GraphConfig, ImdbSourceConfig, LoggingConfig, SourcesConfig,
            SqliteConfig, TmdbSourceConfig,
        },
    };
    use std::io::Write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn graph_projection_builds_and_answers_queries() {
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

        let service = GraphService::open(&config, &paths).expect("graph service");
        let stats = service.rebuild(&db).await.expect("rebuild");
        assert_eq!(stats.titles_projected, 2);
        assert_eq!(stats.people_projected, 2);
        assert_eq!(stats.credits_projected, 4);

        let output = service
            .query(
                "PREFIX schema: <https://schema.org/>
                 SELECT ?title WHERE {
                   ?film schema:director <https://cinegraph.local/person/nm1> .
                   ?film schema:name ?title .
                 }
                 ORDER BY ?title",
            )
            .expect("query");
        let GraphQueryOutput::Solutions { rows, .. } = output else {
            panic!("expected solutions");
        };
        assert_eq!(rows.len(), 2);

        let sparql_path = temp.path().join("directed_films.rq");
        let mut sparql_file = std::fs::File::create(&sparql_path).expect("query file");
        sparql_file
            .write_all(
                b"PREFIX schema: <https://schema.org/>\nSELECT ?title WHERE {\n  ?film schema:director <https://cinegraph.local/person/nm1> .\n  ?film schema:name ?title .\n}\nORDER BY ?title\n",
            )
            .expect("write query");
        let file_output = service.query_file(&sparql_path).expect("query file");
        let GraphQueryOutput::Solutions {
            rows: file_rows, ..
        } = file_output
        else {
            panic!("expected file solutions");
        };
        assert_eq!(file_rows.len(), 2);

        let neighbors = service.neighbors("nm1").expect("neighbors");
        assert!(neighbors.iter().any(|neighbor| neighbor.entity_id == "tt1"));
        assert!(neighbors.iter().any(|neighbor| neighbor.entity_id == "tt2"));

        let collaborations = service.collaborations("nm1").expect("collabs");
        assert_eq!(collaborations.len(), 1);
        assert_eq!(collaborations[0].person_id, "nm2");
        assert_eq!(collaborations[0].shared_titles, 2);
    }
}
