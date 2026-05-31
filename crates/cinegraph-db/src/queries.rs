use cinegraph_core::Result;
use sqlx::Row;

use crate::models::{
    Dataset, DownloadArtifact, IndexablePerson, IndexableTitle, LookupPerson, LookupTitle,
};

pub async fn upsert_dataset(
    pool: &sqlx::SqlitePool,
    source: &str,
    dataset_name: &str,
    canonical_url: &str,
) -> Result<Dataset> {
    sqlx::query(
        r#"
        INSERT INTO datasets (source, dataset_name, canonical_url)
        VALUES (?, ?, ?)
        ON CONFLICT(source, dataset_name) DO UPDATE SET canonical_url = excluded.canonical_url
        "#,
    )
    .bind(source)
    .bind(dataset_name)
    .bind(canonical_url)
    .execute(pool)
    .await?;

    let dataset = sqlx::query_as::<_, Dataset>(
        "SELECT * FROM datasets WHERE source = ? AND dataset_name = ?",
    )
    .bind(source)
    .bind(dataset_name)
    .fetch_one(pool)
    .await?;
    Ok(dataset)
}

pub async fn last_artifact_for_dataset(
    pool: &sqlx::SqlitePool,
    dataset_id: i64,
) -> Result<Option<DownloadArtifact>> {
    let artifact = sqlx::query_as::<_, DownloadArtifact>(
        r#"
        SELECT * FROM download_artifacts
        WHERE dataset_id = ?
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(dataset_id)
    .fetch_optional(pool)
    .await?;
    Ok(artifact)
}

pub async fn artifact_by_hash(
    pool: &sqlx::SqlitePool,
    dataset_id: i64,
    sha256: &str,
) -> Result<Option<DownloadArtifact>> {
    let artifact = sqlx::query_as::<_, DownloadArtifact>(
        "SELECT * FROM download_artifacts WHERE dataset_id = ? AND sha256 = ?",
    )
    .bind(dataset_id)
    .bind(sha256)
    .fetch_optional(pool)
    .await?;
    Ok(artifact)
}

pub async fn insert_artifact(
    pool: &sqlx::SqlitePool,
    dataset_id: i64,
    url: &str,
    local_path: &str,
    sha256: &str,
    byte_len: i64,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<DownloadArtifact> {
    sqlx::query(
        r#"
        INSERT INTO download_artifacts (
            dataset_id, url, local_path, sha256, byte_len, etag, last_modified
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(dataset_id, sha256) DO NOTHING
        "#,
    )
    .bind(dataset_id)
    .bind(url)
    .bind(local_path)
    .bind(sha256)
    .bind(byte_len)
    .bind(etag)
    .bind(last_modified)
    .execute(pool)
    .await?;

    let artifact = sqlx::query_as::<_, DownloadArtifact>(
        "SELECT * FROM download_artifacts WHERE dataset_id = ? AND sha256 = ?",
    )
    .bind(dataset_id)
    .bind(sha256)
    .fetch_one(pool)
    .await?;
    Ok(artifact)
}

pub async fn try_begin_import_run(
    pool: &sqlx::SqlitePool,
    artifact_id: i64,
    importer_name: &str,
    importer_version: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO import_runs (artifact_id, importer_name, importer_version, status)
        VALUES (?, ?, ?, 'running')
        ON CONFLICT(artifact_id, importer_name, importer_version) DO NOTHING
        "#,
    )
    .bind(artifact_id)
    .bind(importer_name)
    .bind(importer_version)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn finish_import_run(
    pool: &sqlx::SqlitePool,
    artifact_id: i64,
    importer_name: &str,
    importer_version: &str,
    status: &str,
    rows_seen: i64,
    rows_inserted: i64,
    rows_updated: i64,
    rows_skipped: i64,
    error_message: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE import_runs
        SET status = ?, finished_at = CURRENT_TIMESTAMP, rows_seen = ?, rows_inserted = ?,
            rows_updated = ?, rows_skipped = ?, error_message = ?
        WHERE artifact_id = ? AND importer_name = ? AND importer_version = ?
        "#,
    )
    .bind(status)
    .bind(rows_seen)
    .bind(rows_inserted)
    .bind(rows_updated)
    .bind(rows_skipped)
    .bind(error_message)
    .bind(artifact_id)
    .bind(importer_name)
    .bind(importer_version)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn latest_artifacts_for_source(
    pool: &sqlx::SqlitePool,
    source: &str,
) -> Result<Vec<(String, DownloadArtifact)>> {
    let rows = sqlx::query(
        r#"
        SELECT d.dataset_name, a.id, a.dataset_id, a.url, a.local_path, a.sha256, a.byte_len, a.etag, a.last_modified, a.fetched_at
        FROM datasets d
        JOIN download_artifacts a ON a.dataset_id = d.id
        WHERE d.source = ?
          AND a.id IN (
            SELECT MAX(a2.id)
            FROM download_artifacts a2
            WHERE a2.dataset_id = d.id
          )
        ORDER BY d.dataset_name
        "#,
    )
    .bind(source)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push((
            row.try_get::<String, _>("dataset_name")?,
            DownloadArtifact {
                id: row.try_get("id")?,
                dataset_id: row.try_get("dataset_id")?,
                url: row.try_get("url")?,
                local_path: row.try_get("local_path")?,
                sha256: row.try_get("sha256")?,
                byte_len: row.try_get("byte_len")?,
                etag: row.try_get("etag")?,
                last_modified: row.try_get("last_modified")?,
                fetched_at: row.try_get("fetched_at")?,
            },
        ));
    }
    Ok(out)
}

pub async fn lookup_title(pool: &sqlx::SqlitePool, query: &str) -> Result<Vec<LookupTitle>> {
    let like = format!("%{query}%");
    let rows = sqlx::query_as::<_, LookupTitle>(
        r#"
        SELECT imdb_id, primary_title, original_title, title_type, start_year, runtime_minutes, genres
        FROM titles
        WHERE imdb_id = ? OR primary_title LIKE ? OR original_title LIKE ?
        ORDER BY start_year, primary_title
        LIMIT 20
        "#,
    )
    .bind(query)
    .bind(&like)
    .bind(&like)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn lookup_person(pool: &sqlx::SqlitePool, query: &str) -> Result<Vec<LookupPerson>> {
    let like = format!("%{query}%");
    let rows = sqlx::query_as::<_, LookupPerson>(
        r#"
        SELECT imdb_name_id, primary_name, birth_year, death_year, primary_professions
        FROM people
        WHERE imdb_name_id = ? OR primary_name LIKE ?
        ORDER BY primary_name
        LIMIT 20
        "#,
    )
    .bind(query)
    .bind(&like)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn titles_for_search_index(pool: &sqlx::SqlitePool) -> Result<Vec<IndexableTitle>> {
    let rows = sqlx::query_as::<_, IndexableTitle>(
        r#"
        SELECT
            t.imdb_id,
            t.primary_title,
            t.original_title,
            t.title_type,
            t.start_year,
            t.genres,
            GROUP_CONCAT(DISTINCT p.primary_name) AS people_text
        FROM titles t
        LEFT JOIN credits c ON c.imdb_id = t.imdb_id
        LEFT JOIN people p ON p.imdb_name_id = c.imdb_name_id
        GROUP BY t.imdb_id, t.primary_title, t.original_title, t.title_type, t.start_year, t.genres
        ORDER BY t.primary_title
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn people_for_search_index(pool: &sqlx::SqlitePool) -> Result<Vec<IndexablePerson>> {
    let rows = sqlx::query_as::<_, IndexablePerson>(
        r#"
        SELECT
            p.imdb_name_id,
            p.primary_name,
            p.birth_year,
            p.death_year,
            p.primary_professions,
            GROUP_CONCAT(DISTINCT t.primary_title) AS title_text
        FROM people p
        LEFT JOIN credits c ON c.imdb_name_id = p.imdb_name_id
        LEFT JOIN titles t ON t.imdb_id = c.imdb_id
        GROUP BY p.imdb_name_id, p.primary_name, p.birth_year, p.death_year, p.primary_professions
        ORDER BY p.primary_name
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn titles_by_ids_in_order(
    pool: &sqlx::SqlitePool,
    ids: &[String],
) -> Result<Vec<LookupTitle>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT imdb_id, primary_title, original_title, title_type, start_year, runtime_minutes, genres
         FROM titles WHERE imdb_id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, LookupTitle>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    let mut by_id = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        by_id.insert(row.imdb_id.clone(), row);
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

pub async fn people_by_ids_in_order(
    pool: &sqlx::SqlitePool,
    ids: &[String],
) -> Result<Vec<LookupPerson>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT imdb_name_id, primary_name, birth_year, death_year, primary_professions
         FROM people WHERE imdb_name_id IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, LookupPerson>(&sql);
    for id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    let mut by_id = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        by_id.insert(row.imdb_name_id.clone(), row);
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

pub async fn stats(pool: &sqlx::SqlitePool) -> Result<serde_json::Value> {
    async fn count(pool: &sqlx::SqlitePool, table: &str) -> Result<i64> {
        let sql = format!("SELECT COUNT(*) as count FROM {table}");
        let row = sqlx::query(&sql).fetch_one(pool).await?;
        Ok(row.get::<i64, _>("count"))
    }

    Ok(serde_json::json!({
        "datasets": count(pool, "datasets").await?,
        "artifacts": count(pool, "download_artifacts").await?,
        "import_runs": count(pool, "import_runs").await?,
        "titles": count(pool, "titles").await?,
        "people": count(pool, "people").await?,
        "title_ratings": count(pool, "title_ratings").await?,
        "title_akas": count(pool, "title_akas").await?,
        "title_crew": count(pool, "title_crew").await?,
        "credits": count(pool, "credits").await?,
        "episode_edges": count(pool, "episode_edges").await?
    }))
}
