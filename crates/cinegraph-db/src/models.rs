use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Dataset {
    pub id: i64,
    pub source: String,
    pub dataset_name: String,
    pub canonical_url: String,
    pub license_note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct DownloadArtifact {
    pub id: i64,
    pub dataset_id: i64,
    pub url: String,
    pub local_path: String,
    pub sha256: String,
    pub byte_len: i64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct LookupTitle {
    pub imdb_id: String,
    pub primary_title: String,
    pub original_title: Option<String>,
    pub title_type: String,
    pub start_year: Option<i64>,
    pub runtime_minutes: Option<i64>,
    pub genres: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct LookupPerson {
    pub imdb_name_id: String,
    pub primary_name: String,
    pub birth_year: Option<i64>,
    pub death_year: Option<i64>,
    pub primary_professions: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct IndexableTitle {
    pub imdb_id: String,
    pub primary_title: String,
    pub original_title: Option<String>,
    pub title_type: String,
    pub start_year: Option<i64>,
    pub genres: Option<String>,
    pub people_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct IndexablePerson {
    pub imdb_name_id: String,
    pub primary_name: String,
    pub birth_year: Option<i64>,
    pub death_year: Option<i64>,
    pub primary_professions: Option<String>,
    pub title_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphTitle {
    pub imdb_id: String,
    pub primary_title: String,
    pub original_title: Option<String>,
    pub title_type: String,
    pub start_year: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphPerson {
    pub imdb_name_id: String,
    pub primary_name: String,
    pub birth_year: Option<i64>,
    pub death_year: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphCredit {
    pub id: i64,
    pub imdb_id: String,
    pub imdb_name_id: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphEpisode {
    pub imdb_id: String,
    pub parent_imdb_id: String,
    pub season_number: Option<i64>,
    pub episode_number: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphRating {
    pub imdb_id: String,
    pub average_rating: Option<f64>,
    pub num_votes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphNeighborRow {
    pub direction: String,
    pub predicate: String,
    pub entity_id: String,
    pub entity_name: String,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphCollaborationRow {
    pub person_id: String,
    pub person_name: String,
    pub shared_titles: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TmdbMovieExportEntry {
    pub id: i64,
    pub export_artifact_id: i64,
    pub tmdb_movie_id: i64,
    pub adult: i64,
    pub original_title: Option<String>,
    pub popularity: Option<f64>,
    pub video: i64,
    pub hydrate_status: Option<String>,
    pub hydrate_attempts: i64,
    pub last_error: Option<String>,
    pub hydrated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PendingTmdbMovieHydration {
    pub export_artifact_id: i64,
    pub tmdb_movie_id: i64,
    pub original_title: Option<String>,
    pub popularity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphWikidataLink {
    pub entity_kind: String,
    pub local_id: String,
    pub local_name: String,
    pub wikidata_id: String,
    pub wikidata_label: Option<String>,
    pub wikidata_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GraphWikidataClaim {
    pub claim_id: i64,
    pub entity_kind: String,
    pub local_id: String,
    pub subject_wikidata_id: String,
    pub property_id: String,
    pub value_type: String,
    pub value_text: Option<String>,
    pub value_wikidata_id: Option<String>,
    pub value_wikidata_label: Option<String>,
}
