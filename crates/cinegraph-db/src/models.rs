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
    pub imdb_id: String,
    pub primary_title: String,
    pub imdb_name_id: String,
    pub primary_name: String,
    pub ordering: Option<i64>,
    pub category: Option<String>,
    pub job: Option<String>,
    pub characters: Option<String>,
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
