use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use cinegraph_core::{AppConfig, AppPaths, CinegraphError, Result};
use cinegraph_db::{Database, queries};
use oxigraph::{
    io::{RdfFormat, RdfSerializer},
    model::{GraphNameRef, Literal, NamedNode, Quad, Subject, Term},
    sparql::QueryResults,
    store::Store,
};
use serde::{Deserialize, Serialize};
use tracing::info;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const SCHEMA_NAME: &str = "https://schema.org/name";
const SCHEMA_PERSON: &str = "https://schema.org/Person";
const SCHEMA_MOVIE: &str = "https://schema.org/Movie";
const SCHEMA_DIRECTOR: &str = "https://schema.org/director";
const SCHEMA_ACTOR: &str = "https://schema.org/actor";
const SCHEMA_DATE_PUBLISHED: &str = "https://schema.org/datePublished";
const SCHEMA_SAME_AS: &str = "https://schema.org/sameAs";
const WIKIDATA_ENTITY_BASE: &str = "https://www.wikidata.org/entity/";
const WIKIDATA_PROPERTY_BASE: &str = "https://www.wikidata.org/prop/direct/";
const PAGE_SIZE: i64 = 50_000;

#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphBuildStats {
    pub titles_projected: usize,
    pub people_projected: usize,
    pub credits_projected: usize,
    pub episode_edges_projected: usize,
    pub ratings_projected: usize,
    pub wikidata_links_projected: usize,
    pub wikidata_claims_projected: usize,
    pub title_triples_written: usize,
    pub person_triples_written: usize,
    pub credit_triples_written: usize,
    pub episode_triples_written: usize,
    pub rating_triples_written: usize,
    pub wikidata_link_triples_written: usize,
    pub wikidata_claim_triples_written: usize,
    pub total_triples_written: usize,
    pub wikidata_entities_projected: usize,
    pub predicate_counts: BTreeMap<String, usize>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStoreStats {
    pub total_triples: usize,
    pub title_nodes: usize,
    pub person_nodes: usize,
    pub wikidata_entities: usize,
    pub predicate_counts: BTreeMap<String, usize>,
    pub store_bytes: u64,
    pub store_path: String,
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
    store_path: PathBuf,
    stats_path: PathBuf,
    temp_dir: PathBuf,
    base_iri: String,
}

impl GraphService {
    pub fn reset_store(paths: &AppPaths) -> Result<()> {
        let graph_dir = paths.graph_store_dir();
        if graph_dir.exists() {
            fs::remove_dir_all(&graph_dir)?;
        }
        fs::create_dir_all(&graph_dir)?;
        Ok(())
    }

    pub fn open(config: &AppConfig, paths: &AppPaths) -> Result<Self> {
        fs::create_dir_all(paths.graph_store_dir())?;
        let store = Store::open(paths.graph_store_dir()).map_err(graph_error)?;
        Ok(Self {
            store,
            store_path: paths.graph_store_dir(),
            stats_path: paths.graph_stats_path(),
            temp_dir: paths.temp_dir(),
            base_iri: config.graph.base_iri.clone(),
        })
    }

    pub async fn rebuild(&self, db: &Database) -> Result<GraphBuildStats> {
        let projection_path = self.temp_dir.join("cinegraph.graph-build.nq");
        if projection_path.exists() {
            fs::remove_file(&projection_path)?;
        }

        let stats = self.write_projection_file(db, &projection_path).await?;
        self.store.clear().map_err(graph_error)?;

        let loader_threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4)
            .max(2);
        let projection_file = File::open(&projection_path)?;

        info!(
            path = %projection_path.display(),
            threads = loader_threads,
            memory_mb = 2048usize,
            "loading projected N-Quads into Oxigraph"
        );
        self.store
            .bulk_loader()
            .with_num_threads(loader_threads)
            .with_max_memory_size_in_megabytes(2048)
            .on_progress(|triples| {
                info!(loaded_triples = triples, "oxigraph bulk load progress");
            })
            .load_from_reader(RdfFormat::NQuads, projection_file)
            .map_err(graph_error)?;

        fs::remove_file(&projection_path)?;

        let summary = GraphStoreStats {
            total_triples: stats.total_triples_written,
            title_nodes: stats.titles_projected,
            person_nodes: stats.people_projected,
            wikidata_entities: stats.wikidata_entities_projected,
            predicate_counts: stats.predicate_counts.clone(),
            store_bytes: dir_size(&self.store_path)?,
            store_path: self.store_path.display().to_string(),
        };
        self.write_stats_summary(&summary)?;

        info!(
            store_path = %self.store_path.display(),
            total_triples_written = stats.total_triples_written,
            titles_projected = stats.titles_projected,
            people_projected = stats.people_projected,
            credits_projected = stats.credits_projected,
            episode_edges_projected = stats.episode_edges_projected,
            ratings_projected = stats.ratings_projected,
            wikidata_links_projected = stats.wikidata_links_projected,
            wikidata_claims_projected = stats.wikidata_claims_projected,
            "lean oxigraph graph rebuilt"
        );

        Ok(stats)
    }

    pub fn query_file(&self, path: &Path) -> Result<GraphQueryOutput> {
        let query = fs::read_to_string(path)?;
        self.query(&query)
    }

    pub fn query(&self, query: &str) -> Result<GraphQueryOutput> {
        let results = self.store.query(query).map_err(graph_error)?;
        graph_query_output(results)
    }

    pub fn stats(&self) -> Result<GraphStoreStats> {
        let raw = fs::read_to_string(&self.stats_path).map_err(|error| {
            CinegraphError::Graph(format!(
                "cached graph stats unavailable at {}: {error}. Rebuild the graph or run `graph stats-heavy`.",
                self.stats_path.display()
            ))
        })?;
        serde_json::from_str(&raw).map_err(graph_error)
    }

    pub fn stats_heavy(&self) -> Result<GraphStoreStats> {
        let total_triples = self.store.len().map_err(graph_error)?;
        let title_nodes = self.count_query(
            "PREFIX schema: <https://schema.org/>
             SELECT (COUNT(DISTINCT ?node) AS ?count) WHERE {
               ?node a schema:Movie .
             }",
        )?;
        let person_nodes = self.count_query(
            "PREFIX schema: <https://schema.org/>
             SELECT (COUNT(DISTINCT ?node) AS ?count) WHERE {
               ?node a schema:Person .
             }",
        )?;
        let wikidata_entities = self.count_query(&format!(
            "PREFIX schema: <https://schema.org/>
             SELECT (COUNT(DISTINCT ?node) AS ?count) WHERE {{
               ?node schema:name ?name .
               FILTER(STRSTARTS(STR(?node), \"{WIKIDATA_ENTITY_BASE}\"))
             }}"
        ))?;

        let predicate_output = self.query(
            "SELECT ?predicate (COUNT(*) AS ?count) WHERE {
               ?subject ?predicate ?object .
             }
             GROUP BY ?predicate
             ORDER BY DESC(?count) STR(?predicate)",
        )?;
        let GraphQueryOutput::Solutions { rows, .. } = predicate_output else {
            return Err(CinegraphError::Graph(
                "graph stats predicate query did not return solutions".to_string(),
            ));
        };
        let mut predicate_counts = BTreeMap::new();
        for row in rows {
            if let (Some(predicate), Some(count)) = (row.get("predicate"), row.get("count")) {
                predicate_counts.insert(
                    compact_iri(predicate),
                    count.parse::<usize>().unwrap_or_default(),
                );
            }
        }

        let stats = GraphStoreStats {
            total_triples,
            title_nodes,
            person_nodes,
            wikidata_entities,
            predicate_counts,
            store_bytes: dir_size(&self.store_path)?,
            store_path: self.store_path.display().to_string(),
        };
        self.write_stats_summary(&stats)?;
        Ok(stats)
    }

    pub async fn neighbors_fast(db: &Database, entity_id: &str) -> Result<Vec<GraphNeighbor>> {
        let rows = if entity_id.starts_with("tt") {
            queries::title_neighbors(db.pool(), entity_id).await?
        } else if entity_id.starts_with("nm") {
            queries::person_neighbors(db.pool(), entity_id).await?
        } else {
            Vec::new()
        };

        Ok(rows
            .into_iter()
            .map(|row| GraphNeighbor {
                direction: row.direction,
                predicate: row.predicate,
                entity_id: row.entity_id,
                entity_name: row.entity_name,
            })
            .collect())
    }

    pub async fn collaborations_fast(
        db: &Database,
        person_id: &str,
    ) -> Result<Vec<CollaborationHit>> {
        let rows = queries::graph_collaborations(db.pool(), person_id).await?;
        Ok(rows
            .into_iter()
            .map(|row| CollaborationHit {
                person_id: row.person_id,
                person_name: row.person_name,
                shared_titles: row.shared_titles.max(0) as usize,
            })
            .collect())
    }

    pub fn neighbors_heavy(&self, entity_id: &str) -> Result<Vec<GraphNeighbor>> {
        let entity_iri = self.entity_iri(entity_id);
        let query = format!(
            "PREFIX schema: <https://schema.org/>
             SELECT ?direction ?predicate ?neighbor ?name WHERE {{
               {{
                 BIND(\"out\" AS ?direction)
                 <{entity_iri}> ?predicate ?neighbor .
                 ?neighbor schema:name ?name .
                 FILTER(
                   STRSTARTS(STR(?neighbor), \"{base}title/\")
                   || STRSTARTS(STR(?neighbor), \"{base}person/\")
                   || STRSTARTS(STR(?neighbor), \"{wikidata_base}\")
                 )
               }}
               UNION
               {{
                 BIND(\"in\" AS ?direction)
                 ?neighbor ?predicate <{entity_iri}> .
                 ?neighbor schema:name ?name .
                 FILTER(
                   STRSTARTS(STR(?neighbor), \"{base}title/\")
                   || STRSTARTS(STR(?neighbor), \"{base}person/\")
                   || STRSTARTS(STR(?neighbor), \"{wikidata_base}\")
                 )
               }}
             }}
             ORDER BY ?direction ?predicate ?name",
            base = self.base_iri,
            wikidata_base = WIKIDATA_ENTITY_BASE
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

    pub fn collaborations_heavy(&self, person_id: &str) -> Result<Vec<CollaborationHit>> {
        let person_iri = self.entity_iri(person_id);
        let query = format!(
            "PREFIX schema: <https://schema.org/>
             PREFIX cine: <{base}>
             SELECT ?other ?name (COUNT(DISTINCT ?title) AS ?shared) WHERE {{
               ?title cine:participant <{person_iri}> .
               ?title cine:participant ?other .
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

    async fn write_projection_file(&self, db: &Database, path: &Path) -> Result<GraphBuildStats> {
        fs::create_dir_all(&self.temp_dir)?;
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        let mut serializer = RdfSerializer::from_format(RdfFormat::NQuads).for_writer(writer);
        let mut stats = GraphBuildStats::default();

        let rdf_type = named_node(RDF_TYPE)?;
        let movie_type = named_node(SCHEMA_MOVIE)?;
        let person_type = named_node(SCHEMA_PERSON)?;
        let schema_name = named_node(SCHEMA_NAME)?;
        let schema_director = named_node(SCHEMA_DIRECTOR)?;
        let schema_actor = named_node(SCHEMA_ACTOR)?;
        let schema_date_published = named_node(SCHEMA_DATE_PUBLISHED)?;
        let schema_same_as = named_node(SCHEMA_SAME_AS)?;
        let participant_predicate = self.cine_predicate("participant")?;
        let title_type_predicate = self.cine_predicate("titleType")?;
        let original_title_predicate = self.cine_predicate("originalTitle")?;
        let writer_predicate = self.cine_predicate("writer")?;
        let producer_predicate = self.cine_predicate("producer")?;
        let birth_year_predicate = self.cine_predicate("birthYear")?;
        let death_year_predicate = self.cine_predicate("deathYear")?;
        let parent_series_predicate = self.cine_predicate("partOfSeries")?;
        let season_number_predicate = self.cine_predicate("seasonNumber")?;
        let episode_number_predicate = self.cine_predicate("episodeNumber")?;
        let average_rating_predicate = self.cine_predicate("averageRating")?;
        let vote_count_predicate = self.cine_predicate("voteCount")?;
        let description_predicate = self.cine_predicate("description")?;
        let mut wikidata_entities = HashSet::new();

        let mut last_title_id = None::<String>;
        loop {
            let rows =
                queries::titles_for_graph_page(db.pool(), last_title_id.as_deref(), PAGE_SIZE)
                    .await?;
            if rows.is_empty() {
                break;
            }
            for title in &rows {
                let title_node = self.title_node(&title.imdb_id)?;
                write_quad(
                    &mut serializer,
                    Quad::new(
                        title_node.clone(),
                        rdf_type.clone(),
                        movie_type.clone(),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.title_triples_written += 1;
                record_predicate(&mut stats, &rdf_type);
                write_quad(
                    &mut serializer,
                    Quad::new(
                        title_node.clone(),
                        schema_name.clone(),
                        Literal::new_simple_literal(&title.primary_title),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.title_triples_written += 1;
                record_predicate(&mut stats, &schema_name);
                write_quad(
                    &mut serializer,
                    Quad::new(
                        title_node.clone(),
                        title_type_predicate.clone(),
                        Literal::new_simple_literal(&title.title_type),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.title_triples_written += 1;
                record_predicate(&mut stats, &title_type_predicate);

                if let Some(year) = title.start_year {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            title_node.clone(),
                            schema_date_published.clone(),
                            Literal::from(year),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.title_triples_written += 1;
                    record_predicate(&mut stats, &schema_date_published);
                }
                if let Some(original_title) = &title.original_title {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            title_node,
                            original_title_predicate.clone(),
                            Literal::new_simple_literal(original_title),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.title_triples_written += 1;
                    record_predicate(&mut stats, &original_title_predicate);
                }
            }
            stats.titles_projected += rows.len();
            last_title_id = rows.last().map(|row| row.imdb_id.clone());
            info!(
                titles_projected = stats.titles_projected,
                title_triples_written = stats.title_triples_written,
                "projected title batch"
            );
        }

        let mut last_person_id = None::<String>;
        loop {
            let rows =
                queries::people_for_graph_page(db.pool(), last_person_id.as_deref(), PAGE_SIZE)
                    .await?;
            if rows.is_empty() {
                break;
            }
            for person in &rows {
                let person_node = self.person_node(&person.imdb_name_id)?;
                write_quad(
                    &mut serializer,
                    Quad::new(
                        person_node.clone(),
                        rdf_type.clone(),
                        person_type.clone(),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.person_triples_written += 1;
                record_predicate(&mut stats, &rdf_type);
                write_quad(
                    &mut serializer,
                    Quad::new(
                        person_node.clone(),
                        schema_name.clone(),
                        Literal::new_simple_literal(&person.primary_name),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.person_triples_written += 1;
                record_predicate(&mut stats, &schema_name);
                if let Some(year) = person.birth_year {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            person_node.clone(),
                            birth_year_predicate.clone(),
                            Literal::from(year),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.person_triples_written += 1;
                    record_predicate(&mut stats, &birth_year_predicate);
                }
                if let Some(year) = person.death_year {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            person_node,
                            death_year_predicate.clone(),
                            Literal::from(year),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.person_triples_written += 1;
                    record_predicate(&mut stats, &death_year_predicate);
                }
            }
            stats.people_projected += rows.len();
            last_person_id = rows.last().map(|row| row.imdb_name_id.clone());
            info!(
                people_projected = stats.people_projected,
                person_triples_written = stats.person_triples_written,
                "projected person batch"
            );
        }

        let mut last_credit_id = 0_i64;
        loop {
            let rows =
                queries::credits_for_graph_page(db.pool(), last_credit_id, PAGE_SIZE).await?;
            if rows.is_empty() {
                break;
            }
            for credit in &rows {
                let title_node = self.title_node(&credit.imdb_id)?;
                let person_node = self.person_node(&credit.imdb_name_id)?;
                write_quad(
                    &mut serializer,
                    Quad::new(
                        title_node.clone(),
                        participant_predicate.clone(),
                        person_node.clone(),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.credit_triples_written += 1;
                record_predicate(&mut stats, &participant_predicate);

                match credit.category.as_deref() {
                    Some("director") => {
                        write_quad(
                            &mut serializer,
                            Quad::new(
                                title_node,
                                schema_director.clone(),
                                person_node,
                                GraphNameRef::DefaultGraph,
                            ),
                        )?;
                        stats.credit_triples_written += 1;
                        record_predicate(&mut stats, &schema_director);
                    }
                    Some("actor") | Some("actress") => {
                        write_quad(
                            &mut serializer,
                            Quad::new(
                                title_node,
                                schema_actor.clone(),
                                person_node,
                                GraphNameRef::DefaultGraph,
                            ),
                        )?;
                        stats.credit_triples_written += 1;
                        record_predicate(&mut stats, &schema_actor);
                    }
                    Some("writer") => {
                        write_quad(
                            &mut serializer,
                            Quad::new(
                                title_node,
                                writer_predicate.clone(),
                                person_node,
                                GraphNameRef::DefaultGraph,
                            ),
                        )?;
                        stats.credit_triples_written += 1;
                        record_predicate(&mut stats, &writer_predicate);
                    }
                    Some("producer") => {
                        write_quad(
                            &mut serializer,
                            Quad::new(
                                title_node,
                                producer_predicate.clone(),
                                person_node,
                                GraphNameRef::DefaultGraph,
                            ),
                        )?;
                        stats.credit_triples_written += 1;
                        record_predicate(&mut stats, &producer_predicate);
                    }
                    _ => {}
                }
            }
            stats.credits_projected += rows.len();
            last_credit_id = rows.last().map(|row| row.id).unwrap_or(last_credit_id);
            info!(
                credits_projected = stats.credits_projected,
                credit_triples_written = stats.credit_triples_written,
                "projected credit batch"
            );
        }

        let mut last_episode_id = None::<String>;
        loop {
            let rows = queries::episode_edges_for_graph_page(
                db.pool(),
                last_episode_id.as_deref(),
                PAGE_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            for episode in &rows {
                let episode_node = self.title_node(&episode.imdb_id)?;
                let parent_node = self.title_node(&episode.parent_imdb_id)?;
                write_quad(
                    &mut serializer,
                    Quad::new(
                        episode_node.clone(),
                        parent_series_predicate.clone(),
                        parent_node,
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.episode_triples_written += 1;
                record_predicate(&mut stats, &parent_series_predicate);
                if let Some(season) = episode.season_number {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            episode_node.clone(),
                            season_number_predicate.clone(),
                            Literal::from(season),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.episode_triples_written += 1;
                    record_predicate(&mut stats, &season_number_predicate);
                }
                if let Some(number) = episode.episode_number {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            episode_node,
                            episode_number_predicate.clone(),
                            Literal::from(number),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.episode_triples_written += 1;
                    record_predicate(&mut stats, &episode_number_predicate);
                }
            }
            stats.episode_edges_projected += rows.len();
            last_episode_id = rows.last().map(|row| row.imdb_id.clone());
            info!(
                episode_edges_projected = stats.episode_edges_projected,
                episode_triples_written = stats.episode_triples_written,
                "projected episode batch"
            );
        }

        let mut last_rating_id = None::<String>;
        loop {
            let rows = queries::title_ratings_for_graph_page(
                db.pool(),
                last_rating_id.as_deref(),
                PAGE_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            for rating in &rows {
                let title_node = self.title_node(&rating.imdb_id)?;
                if let Some(value) = rating.average_rating {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            title_node.clone(),
                            average_rating_predicate.clone(),
                            Literal::from(value),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.rating_triples_written += 1;
                    record_predicate(&mut stats, &average_rating_predicate);
                }
                if let Some(votes) = rating.num_votes {
                    write_quad(
                        &mut serializer,
                        Quad::new(
                            title_node,
                            vote_count_predicate.clone(),
                            Literal::from(votes),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.rating_triples_written += 1;
                    record_predicate(&mut stats, &vote_count_predicate);
                }
            }
            stats.ratings_projected += rows.len();
            last_rating_id = rows.last().map(|row| row.imdb_id.clone());
            info!(
                ratings_projected = stats.ratings_projected,
                rating_triples_written = stats.rating_triples_written,
                "projected ratings batch"
            );
        }

        let mut last_title_link = None::<String>;
        loop {
            let rows = queries::title_wikidata_links_for_graph_page(
                db.pool(),
                last_title_link.as_deref(),
                PAGE_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            self.write_wikidata_link_batch(
                &mut serializer,
                &schema_same_as,
                &schema_name,
                &description_predicate,
                &rows,
                &mut wikidata_entities,
                &mut stats,
            )?;
            last_title_link = rows.last().map(|row| row.local_id.clone());
        }

        let mut last_person_link = None::<String>;
        loop {
            let rows = queries::person_wikidata_links_for_graph_page(
                db.pool(),
                last_person_link.as_deref(),
                PAGE_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            self.write_wikidata_link_batch(
                &mut serializer,
                &schema_same_as,
                &schema_name,
                &description_predicate,
                &rows,
                &mut wikidata_entities,
                &mut stats,
            )?;
            last_person_link = rows.last().map(|row| row.local_id.clone());
        }

        let mut last_title_claim_id = 0_i64;
        loop {
            let rows = queries::title_wikidata_claims_for_graph_page(
                db.pool(),
                last_title_claim_id,
                PAGE_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            self.write_wikidata_claim_batch(
                &mut serializer,
                &schema_name,
                &rows,
                &mut wikidata_entities,
                &mut stats,
            )?;
            last_title_claim_id = rows
                .last()
                .map(|row| row.claim_id)
                .unwrap_or(last_title_claim_id);
        }

        let mut last_person_claim_id = 0_i64;
        loop {
            let rows = queries::person_wikidata_claims_for_graph_page(
                db.pool(),
                last_person_claim_id,
                PAGE_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            self.write_wikidata_claim_batch(
                &mut serializer,
                &schema_name,
                &rows,
                &mut wikidata_entities,
                &mut stats,
            )?;
            last_person_claim_id = rows
                .last()
                .map(|row| row.claim_id)
                .unwrap_or(last_person_claim_id);
        }

        let mut writer = serializer.finish().map_err(graph_error)?;
        writer.flush()?;

        stats.total_triples_written = stats.title_triples_written
            + stats.person_triples_written
            + stats.credit_triples_written
            + stats.episode_triples_written
            + stats.rating_triples_written
            + stats.wikidata_link_triples_written
            + stats.wikidata_claim_triples_written;
        stats.wikidata_entities_projected = wikidata_entities.len();

        Ok(stats)
    }

    fn write_wikidata_link_batch<W: Write>(
        &self,
        serializer: &mut oxigraph::io::WriterQuadSerializer<W>,
        schema_same_as: &NamedNode,
        schema_name: &NamedNode,
        description_predicate: &NamedNode,
        rows: &[cinegraph_db::models::GraphWikidataLink],
        wikidata_entities: &mut HashSet<String>,
        stats: &mut GraphBuildStats,
    ) -> Result<()> {
        for link in rows {
            let local_node = self.local_node(&link.entity_kind, &link.local_id)?;
            let wikidata_node = self.wikidata_node(&link.wikidata_id)?;
            wikidata_entities.insert(link.wikidata_id.clone());
            write_quad(
                serializer,
                Quad::new(
                    local_node,
                    schema_same_as.clone(),
                    wikidata_node.clone(),
                    GraphNameRef::DefaultGraph,
                ),
            )?;
            stats.wikidata_link_triples_written += 1;
            record_predicate(stats, schema_same_as);

            if let Some(label) = &link.wikidata_label {
                write_quad(
                    serializer,
                    Quad::new(
                        wikidata_node.clone(),
                        schema_name.clone(),
                        Literal::new_simple_literal(label),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.wikidata_link_triples_written += 1;
                record_predicate(stats, schema_name);
            }
            if let Some(description) = &link.wikidata_description {
                write_quad(
                    serializer,
                    Quad::new(
                        wikidata_node,
                        description_predicate.clone(),
                        Literal::new_simple_literal(description),
                        GraphNameRef::DefaultGraph,
                    ),
                )?;
                stats.wikidata_link_triples_written += 1;
                record_predicate(stats, description_predicate);
            }
        }
        stats.wikidata_links_projected += rows.len();
        info!(
            wikidata_links_projected = stats.wikidata_links_projected,
            wikidata_link_triples_written = stats.wikidata_link_triples_written,
            "projected wikidata link batch"
        );
        Ok(())
    }

    fn write_wikidata_claim_batch<W: Write>(
        &self,
        serializer: &mut oxigraph::io::WriterQuadSerializer<W>,
        schema_name: &NamedNode,
        rows: &[cinegraph_db::models::GraphWikidataClaim],
        wikidata_entities: &mut HashSet<String>,
        stats: &mut GraphBuildStats,
    ) -> Result<()> {
        for claim in rows {
            let subject = self.local_node(&claim.entity_kind, &claim.local_id)?;
            let predicate = self.wikidata_property_node(&claim.property_id)?;
            let object: Term = if let Some(wikidata_id) = &claim.value_wikidata_id {
                let wikidata_node = self.wikidata_node(wikidata_id)?;
                wikidata_entities.insert(wikidata_id.clone());
                if let Some(label) = &claim.value_wikidata_label {
                    write_quad(
                        serializer,
                        Quad::new(
                            wikidata_node.clone(),
                            schema_name.clone(),
                            Literal::new_simple_literal(label),
                            GraphNameRef::DefaultGraph,
                        ),
                    )?;
                    stats.wikidata_claim_triples_written += 1;
                    record_predicate(stats, schema_name);
                }
                wikidata_node.into()
            } else if let Some(text) = &claim.value_text {
                Literal::new_simple_literal(text).into()
            } else {
                continue;
            };

            write_quad(
                serializer,
                Quad::new(
                    subject,
                    predicate.clone(),
                    object,
                    GraphNameRef::DefaultGraph,
                ),
            )?;
            stats.wikidata_claim_triples_written += 1;
            record_predicate(stats, &predicate);
        }
        stats.wikidata_claims_projected += rows.len();
        info!(
            wikidata_claims_projected = stats.wikidata_claims_projected,
            wikidata_claim_triples_written = stats.wikidata_claim_triples_written,
            "projected wikidata claim batch"
        );
        Ok(())
    }

    fn count_query(&self, query: &str) -> Result<usize> {
        let output = self.query(query)?;
        let GraphQueryOutput::Solutions { rows, .. } = output else {
            return Err(CinegraphError::Graph(
                "count query did not return solutions".to_string(),
            ));
        };
        let count = rows
            .first()
            .and_then(|row| row.get("count"))
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or_default();
        Ok(count)
    }

    fn write_stats_summary(&self, stats: &GraphStoreStats) -> Result<()> {
        if let Some(parent) = self.stats_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.stats_path, serde_json::to_vec_pretty(stats)?)?;
        Ok(())
    }

    fn title_node(&self, imdb_id: &str) -> Result<NamedNode> {
        named_node(&format!("{}title/{imdb_id}", self.base_iri))
    }

    fn person_node(&self, imdb_name_id: &str) -> Result<NamedNode> {
        named_node(&format!("{}person/{imdb_name_id}", self.base_iri))
    }

    fn local_node(&self, entity_kind: &str, local_id: &str) -> Result<NamedNode> {
        match entity_kind {
            "title" => self.title_node(local_id),
            "person" => self.person_node(local_id),
            _ => Err(CinegraphError::Graph(format!(
                "unsupported local entity kind: {entity_kind}"
            ))),
        }
    }

    fn wikidata_node(&self, wikidata_id: &str) -> Result<NamedNode> {
        named_node(&format!("{WIKIDATA_ENTITY_BASE}{wikidata_id}"))
    }

    fn wikidata_property_node(&self, property_id: &str) -> Result<NamedNode> {
        named_node(&format!("{WIKIDATA_PROPERTY_BASE}{property_id}"))
    }

    fn cine_predicate(&self, suffix: &str) -> Result<NamedNode> {
        named_node(&format!("{}{}", self.base_iri, sanitize(suffix)))
    }

    fn entity_iri(&self, entity_id: &str) -> String {
        if entity_id.starts_with("http://") || entity_id.starts_with("https://") {
            entity_id.to_string()
        } else if entity_id.starts_with('Q') {
            format!("{WIKIDATA_ENTITY_BASE}{entity_id}")
        } else if entity_id.starts_with("tt") {
            format!("{}title/{entity_id}", self.base_iri)
        } else {
            format!("{}person/{entity_id}", self.base_iri)
        }
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
                    (
                        "predicate".to_string(),
                        triple.predicate.as_str().to_string(),
                    ),
                    ("object".to_string(), term_value(&triple.object)),
                ]));
            }
            Ok(GraphQueryOutput::Graph { triples: rows })
        }
        QueryResults::Boolean(value) => Ok(GraphQueryOutput::Boolean { value }),
    }
}

fn write_quad<W: Write>(
    serializer: &mut oxigraph::io::WriterQuadSerializer<W>,
    quad: Quad,
) -> Result<()> {
    serializer.serialize_quad(&quad).map_err(graph_error)?;
    Ok(())
}

fn record_predicate(stats: &mut GraphBuildStats, predicate: &NamedNode) {
    *stats
        .predicate_counts
        .entry(compact_iri(predicate.as_str()))
        .or_default() += 1;
}

fn dir_size(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }

    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total += dir_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
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
            SqliteConfig, TmdbSourceConfig, WikidataSourceConfig,
        },
    };
    use tempfile::tempdir;

    #[tokio::test]
    async fn graph_projection_builds_and_answers_queries() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join(".cache/cinegraph");
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
                    enabled: false,
                    dump_path: String::new(),
                    language: "en".to_string(),
                },
            },
        };
        let paths = AppPaths::from_config(&config);
        paths.ensure_dirs(&config).expect("dirs");
        GraphService::reset_store(&paths).expect("reset store");
        let db = Database::connect(&config, &paths).await.expect("db");
        db.migrate().await.expect("migrate");

        sqlx::query("INSERT INTO titles (imdb_id, title_type, primary_title, original_title, is_adult, start_year, genres) VALUES ('tt1', 'movie', 'Seven Samurai', 'Shichinin no samurai', 0, 1954, 'Drama'), ('tt2', 'movie', 'Ikiru', 'Ikiru', 0, 1952, 'Drama'), ('tt3', 'tvEpisode', 'A Bold Episode', 'A Bold Episode', 0, 1955, 'Drama')")
            .execute(db.pool())
            .await
            .expect("titles");
        sqlx::query("INSERT INTO people (imdb_name_id, primary_name, birth_year, primary_professions) VALUES ('nm1', 'Akira Kurosawa', 1910, 'director,writer'), ('nm2', 'Takashi Shimura', 1905, 'actor')")
            .execute(db.pool())
            .await
            .expect("people");
        sqlx::query("INSERT INTO credits (imdb_id, imdb_name_id, ordering, category, source) VALUES ('tt1', 'nm1', 1, 'director', 'imdb'), ('tt1', 'nm2', 2, 'actor', 'imdb'), ('tt2', 'nm1', 1, 'director', 'imdb'), ('tt2', 'nm2', 2, 'actor', 'imdb'), ('tt3', 'nm1', 1, 'writer', 'imdb')")
            .execute(db.pool())
            .await
            .expect("credits");
        sqlx::query("INSERT INTO episode_edges (imdb_id, parent_imdb_id, season_number, episode_number) VALUES ('tt3', 'tt1', 1, 3)")
            .execute(db.pool())
            .await
            .expect("episode edge");
        sqlx::query("INSERT INTO title_ratings (imdb_id, average_rating, num_votes) VALUES ('tt1', 8.6, 1000)")
            .execute(db.pool())
            .await
            .expect("ratings");
        sqlx::query("INSERT INTO wikidata_entities (wikidata_id, label, description, entity_type) VALUES ('Q2000', 'Seven Samurai', 'Wikidata item for Seven Samurai', 'item'), ('Q1000', 'Akira Kurosawa', 'Wikidata item for Akira Kurosawa', 'item'), ('Q2001', 'jidaigeki', 'Japanese period drama genre', 'item')")
            .execute(db.pool())
            .await
            .expect("wikidata entities");
        sqlx::query(
            "INSERT INTO title_wikidata_links (imdb_id, wikidata_id) VALUES ('tt1', 'Q2000')",
        )
        .execute(db.pool())
        .await
        .expect("title wikidata link");
        sqlx::query(
            "INSERT INTO person_wikidata_links (imdb_name_id, wikidata_id) VALUES ('nm1', 'Q1000')",
        )
        .execute(db.pool())
        .await
        .expect("person wikidata link");
        sqlx::query("INSERT INTO wikidata_claims (subject_wikidata_id, property_id, value_type, value_text, value_wikidata_id, ordinal) VALUES ('Q2000', 'P136', 'wikidata_item', NULL, 'Q2001', 0), ('Q2000', 'P577', 'time', '+1954-04-26T00:00:00Z', NULL, 0), ('Q1000', 'P569', 'time', '+1910-03-23T00:00:00Z', NULL, 0)")
            .execute(db.pool())
            .await
            .expect("wikidata claims");

        let service = GraphService::open(&config, &paths).expect("graph service");
        let stats = service.rebuild(&db).await.expect("rebuild");
        assert_eq!(stats.titles_projected, 3);
        assert_eq!(stats.people_projected, 2);
        assert_eq!(stats.credits_projected, 5);
        assert_eq!(stats.episode_edges_projected, 1);
        assert_eq!(stats.ratings_projected, 1);
        assert_eq!(stats.wikidata_links_projected, 2);
        assert_eq!(stats.wikidata_claims_projected, 3);
        assert!(stats.total_triples_written < 50);

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

        let neighbors = GraphService::neighbors_fast(&db, "nm1")
            .await
            .expect("neighbors");
        assert!(neighbors.iter().any(|neighbor| neighbor.entity_id == "tt1"));
        assert!(neighbors.iter().any(|neighbor| neighbor.entity_id == "tt2"));
        assert!(neighbors.iter().any(|neighbor| neighbor.entity_id == "tt3"));
        assert!(
            neighbors
                .iter()
                .any(|neighbor| neighbor.entity_id == "Q1000")
        );

        let title_neighbors = GraphService::neighbors_fast(&db, "tt1")
            .await
            .expect("title neighbors");
        assert!(
            title_neighbors
                .iter()
                .any(|neighbor| neighbor.entity_id == "Q2000" && neighbor.predicate == "sameAs")
        );
        assert!(
            title_neighbors
                .iter()
                .any(|neighbor| neighbor.entity_id == "tt3" && neighbor.predicate == "partOfSeries")
        );
        assert!(
            title_neighbors.iter().any(
                |neighbor| neighbor.predicate == "averageRating" && neighbor.entity_id == "8.6"
            )
        );

        let heavy_neighbors = service.neighbors_heavy("nm1").expect("heavy neighbors");
        assert!(
            heavy_neighbors
                .iter()
                .any(|neighbor| neighbor.entity_id == "tt1")
        );

        let collaborations = GraphService::collaborations_fast(&db, "nm1")
            .await
            .expect("collabs");
        assert_eq!(collaborations.len(), 1);
        assert_eq!(collaborations[0].person_id, "nm2");
        assert_eq!(collaborations[0].shared_titles, 2);

        let heavy_collaborations = service
            .collaborations_heavy("nm1")
            .expect("heavy collaborations");
        assert_eq!(heavy_collaborations.len(), 1);
        assert_eq!(heavy_collaborations[0].person_id, "nm2");
        assert_eq!(heavy_collaborations[0].shared_titles, 2);

        let graph_stats = service.stats().expect("graph stats");
        assert_eq!(graph_stats.title_nodes, 3);
        assert_eq!(graph_stats.person_nodes, 2);
        assert_eq!(graph_stats.total_triples, stats.total_triples_written);
        assert!(graph_stats.store_bytes > 0);
        assert_eq!(
            graph_stats.predicate_counts.get("participant").copied(),
            Some(5)
        );

        let heavy_stats = service.stats_heavy().expect("graph heavy stats");
        assert_eq!(heavy_stats.total_triples, graph_stats.total_triples);
        assert_eq!(
            heavy_stats.predicate_counts.get("participant").copied(),
            Some(5)
        );
    }
}
