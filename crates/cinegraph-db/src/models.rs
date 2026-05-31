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
