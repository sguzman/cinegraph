use cinegraph_core::Result;
use sqlx::Row;

use crate::models::{
    Dataset, DownloadArtifact, GraphCollaborationRow, GraphCredit, GraphEpisode, GraphNeighborRow,
    GraphPerson, GraphRating, GraphTitle, GraphWikidataClaim, GraphWikidataLink, IndexablePerson,
    IndexableTitle, LookupPerson, LookupTitle, PendingTmdbMovieHydration, TmdbMovieExportEntry,
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
    if result.rows_affected() == 1 {
        return Ok(true);
    }

    let existing: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT status
        FROM import_runs
        WHERE artifact_id = ? AND importer_name = ? AND importer_version = ?
        "#,
    )
    .bind(artifact_id)
    .bind(importer_name)
    .bind(importer_version)
    .fetch_optional(pool)
    .await?;

    if matches!(existing.as_ref().map(|row| row.0.as_str()), Some("failed")) {
        sqlx::query(
            r#"
            UPDATE import_runs
            SET status = 'running',
                started_at = CURRENT_TIMESTAMP,
                finished_at = NULL,
                rows_seen = 0,
                rows_inserted = 0,
                rows_updated = 0,
                rows_skipped = 0,
                error_message = NULL
            WHERE artifact_id = ? AND importer_name = ? AND importer_version = ?
            "#,
        )
        .bind(artifact_id)
        .bind(importer_name)
        .bind(importer_version)
        .execute(pool)
        .await?;
        return Ok(true);
    }

    Ok(false)
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

pub async fn latest_artifact_for_source(
    pool: &sqlx::SqlitePool,
    source: &str,
) -> Result<Option<(String, DownloadArtifact)>> {
    let row = sqlx::query(
        r#"
        SELECT d.dataset_name, a.id, a.dataset_id, a.url, a.local_path, a.sha256, a.byte_len, a.etag, a.last_modified, a.fetched_at
        FROM datasets d
        JOIN download_artifacts a ON a.dataset_id = d.id
        WHERE d.source = ?
        ORDER BY a.id DESC
        LIMIT 1
        "#,
    )
    .bind(source)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| {
        (
            row.get::<String, _>("dataset_name"),
            DownloadArtifact {
                id: row.get("id"),
                dataset_id: row.get("dataset_id"),
                url: row.get("url"),
                local_path: row.get("local_path"),
                sha256: row.get("sha256"),
                byte_len: row.get("byte_len"),
                etag: row.get("etag"),
                last_modified: row.get("last_modified"),
                fetched_at: row.get("fetched_at"),
            },
        )
    }))
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

    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(", ");
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

    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(", ");
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

pub async fn titles_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_imdb_id: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphTitle>> {
    let rows = sqlx::query_as::<_, GraphTitle>(
        r#"
        SELECT imdb_id, primary_title, original_title, title_type, start_year
        FROM titles
        WHERE (? IS NULL OR imdb_id > ?)
        ORDER BY imdb_id
        LIMIT ?
        "#,
    )
    .bind(last_imdb_id)
    .bind(last_imdb_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn people_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_imdb_name_id: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphPerson>> {
    let rows = sqlx::query_as::<_, GraphPerson>(
        r#"
        SELECT imdb_name_id, primary_name, birth_year, death_year
        FROM people
        WHERE (? IS NULL OR imdb_name_id > ?)
        ORDER BY imdb_name_id
        LIMIT ?
        "#,
    )
    .bind(last_imdb_name_id)
    .bind(last_imdb_name_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn credits_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_credit_id: i64,
    limit: i64,
) -> Result<Vec<GraphCredit>> {
    let rows = sqlx::query_as::<_, GraphCredit>(
        r#"
        SELECT
            c.id,
            c.imdb_id,
            c.imdb_name_id,
            c.category
        FROM credits c
        WHERE c.id > ?
        ORDER BY c.id
        LIMIT ?
        "#,
    )
    .bind(last_credit_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn episode_edges_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_imdb_id: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphEpisode>> {
    let rows = sqlx::query_as::<_, GraphEpisode>(
        r#"
        SELECT imdb_id, parent_imdb_id, season_number, episode_number
        FROM episode_edges
        WHERE (? IS NULL OR imdb_id > ?)
        ORDER BY imdb_id
        LIMIT ?
        "#,
    )
    .bind(last_imdb_id)
    .bind(last_imdb_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn title_ratings_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_imdb_id: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphRating>> {
    let rows = sqlx::query_as::<_, GraphRating>(
        r#"
        SELECT imdb_id, average_rating, num_votes
        FROM title_ratings
        WHERE (? IS NULL OR imdb_id > ?)
        ORDER BY imdb_id
        LIMIT ?
        "#,
    )
    .bind(last_imdb_id)
    .bind(last_imdb_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn title_neighbors(
    pool: &sqlx::SqlitePool,
    imdb_id: &str,
) -> Result<Vec<GraphNeighborRow>> {
    let rows = sqlx::query_as::<_, GraphNeighborRow>(
        r#"
        SELECT DISTINCT direction, predicate, entity_id, entity_name
        FROM (
            SELECT
                'out' AS direction,
                'participant' AS predicate,
                p.imdb_name_id AS entity_id,
                p.primary_name AS entity_name,
                p.primary_name AS sort_name
            FROM credits c
            JOIN people p ON p.imdb_name_id = c.imdb_name_id
            WHERE c.imdb_id = ?

            UNION ALL

            SELECT
                'out' AS direction,
                CASE c.category
                    WHEN 'director' THEN 'director'
                    WHEN 'actor' THEN 'actor'
                    WHEN 'actress' THEN 'actor'
                    WHEN 'writer' THEN 'writer'
                    WHEN 'producer' THEN 'producer'
                END AS predicate,
                p.imdb_name_id AS entity_id,
                p.primary_name AS entity_name,
                p.primary_name AS sort_name
            FROM credits c
            JOIN people p ON p.imdb_name_id = c.imdb_name_id
            WHERE c.imdb_id = ?
              AND c.category IN ('director', 'actor', 'actress', 'writer', 'producer')

            UNION ALL

            SELECT
                'out' AS direction,
                'partOfSeries' AS predicate,
                t.imdb_id AS entity_id,
                t.primary_title AS entity_name,
                t.primary_title AS sort_name
            FROM episode_edges e
            JOIN titles t ON t.imdb_id = e.parent_imdb_id
            WHERE e.imdb_id = ?

            UNION ALL

            SELECT
                'in' AS direction,
                'partOfSeries' AS predicate,
                t.imdb_id AS entity_id,
                t.primary_title AS entity_name,
                t.primary_title AS sort_name
            FROM episode_edges e
            JOIN titles t ON t.imdb_id = e.imdb_id
            WHERE e.parent_imdb_id = ?

            UNION ALL

            SELECT
                'out' AS direction,
                'sameAs' AS predicate,
                l.wikidata_id AS entity_id,
                COALESCE(e.label, l.wikidata_id) AS entity_name,
                COALESCE(e.label, l.wikidata_id) AS sort_name
            FROM title_wikidata_links l
            LEFT JOIN wikidata_entities e ON e.wikidata_id = l.wikidata_id
            WHERE l.imdb_id = ?

            UNION ALL

            SELECT
                'out' AS direction,
                'averageRating' AS predicate,
                CAST(r.average_rating AS TEXT) AS entity_id,
                CAST(r.average_rating AS TEXT) AS entity_name,
                CAST(r.average_rating AS TEXT) AS sort_name
            FROM title_ratings r
            WHERE r.imdb_id = ?
              AND r.average_rating IS NOT NULL

            UNION ALL

            SELECT
                'out' AS direction,
                'voteCount' AS predicate,
                CAST(r.num_votes AS TEXT) AS entity_id,
                CAST(r.num_votes AS TEXT) AS entity_name,
                CAST(r.num_votes AS TEXT) AS sort_name
            FROM title_ratings r
            WHERE r.imdb_id = ?
              AND r.num_votes IS NOT NULL
        )
        ORDER BY direction, predicate, sort_name, entity_id
        "#,
    )
    .bind(imdb_id)
    .bind(imdb_id)
    .bind(imdb_id)
    .bind(imdb_id)
    .bind(imdb_id)
    .bind(imdb_id)
    .bind(imdb_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn person_neighbors(
    pool: &sqlx::SqlitePool,
    imdb_name_id: &str,
) -> Result<Vec<GraphNeighborRow>> {
    let rows = sqlx::query_as::<_, GraphNeighborRow>(
        r#"
        SELECT DISTINCT direction, predicate, entity_id, entity_name
        FROM (
            SELECT
                'in' AS direction,
                'participant' AS predicate,
                t.imdb_id AS entity_id,
                t.primary_title AS entity_name,
                t.primary_title AS sort_name
            FROM credits c
            JOIN titles t ON t.imdb_id = c.imdb_id
            WHERE c.imdb_name_id = ?

            UNION ALL

            SELECT
                'in' AS direction,
                CASE c.category
                    WHEN 'director' THEN 'director'
                    WHEN 'actor' THEN 'actor'
                    WHEN 'actress' THEN 'actor'
                    WHEN 'writer' THEN 'writer'
                    WHEN 'producer' THEN 'producer'
                END AS predicate,
                t.imdb_id AS entity_id,
                t.primary_title AS entity_name,
                t.primary_title AS sort_name
            FROM credits c
            JOIN titles t ON t.imdb_id = c.imdb_id
            WHERE c.imdb_name_id = ?
              AND c.category IN ('director', 'actor', 'actress', 'writer', 'producer')

            UNION ALL

            SELECT
                'out' AS direction,
                'sameAs' AS predicate,
                l.wikidata_id AS entity_id,
                COALESCE(e.label, l.wikidata_id) AS entity_name,
                COALESCE(e.label, l.wikidata_id) AS sort_name
            FROM person_wikidata_links l
            LEFT JOIN wikidata_entities e ON e.wikidata_id = l.wikidata_id
            WHERE l.imdb_name_id = ?
        )
        ORDER BY direction, predicate, sort_name, entity_id
        "#,
    )
    .bind(imdb_name_id)
    .bind(imdb_name_id)
    .bind(imdb_name_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn graph_collaborations(
    pool: &sqlx::SqlitePool,
    imdb_name_id: &str,
) -> Result<Vec<GraphCollaborationRow>> {
    let rows = sqlx::query_as::<_, GraphCollaborationRow>(
        r#"
        SELECT
            other.imdb_name_id AS person_id,
            p.primary_name AS person_name,
            COUNT(DISTINCT base.imdb_id) AS shared_titles
        FROM credits base
        JOIN credits other
          ON other.imdb_id = base.imdb_id
         AND other.imdb_name_id != base.imdb_name_id
        JOIN people p ON p.imdb_name_id = other.imdb_name_id
        WHERE base.imdb_name_id = ?
        GROUP BY other.imdb_name_id, p.primary_name
        ORDER BY shared_titles DESC, p.primary_name ASC, other.imdb_name_id ASC
        "#,
    )
    .bind(imdb_name_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
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
        "episode_edges": count(pool, "episode_edges").await?,
        "tmdb_movie_exports": count(pool, "tmdb_movie_exports").await?,
        "tmdb_movies": count(pool, "tmdb_movies").await?,
        "tmdb_people": count(pool, "tmdb_people").await?,
        "tmdb_movie_credits": count(pool, "tmdb_movie_credits").await?,
        "title_tmdb_links": count(pool, "title_tmdb_links").await?,
        "wikidata_entities": count(pool, "wikidata_entities").await?,
        "title_wikidata_links": count(pool, "title_wikidata_links").await?,
        "person_wikidata_links": count(pool, "person_wikidata_links").await?,
        "wikidata_claims": count(pool, "wikidata_claims").await?
    }))
}

pub async fn upsert_tmdb_movie_export(
    pool: &sqlx::SqlitePool,
    export_artifact_id: i64,
    tmdb_movie_id: i64,
    adult: bool,
    original_title: Option<&str>,
    popularity: Option<f64>,
    video: bool,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO tmdb_movie_exports (export_artifact_id, tmdb_movie_id, adult, original_title, popularity, video)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(export_artifact_id, tmdb_movie_id) DO UPDATE SET
            adult = excluded.adult,
            original_title = excluded.original_title,
            popularity = excluded.popularity,
            video = excluded.video
        "#,
    )
    .bind(export_artifact_id)
    .bind(tmdb_movie_id)
    .bind(if adult { 1 } else { 0 })
    .bind(original_title)
    .bind(popularity)
    .bind(if video { 1 } else { 0 })
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn pending_tmdb_movie_hydrations(
    pool: &sqlx::SqlitePool,
    export_artifact_id: i64,
    include_adult: bool,
    limit: i64,
) -> Result<Vec<PendingTmdbMovieHydration>> {
    let rows = sqlx::query_as::<_, PendingTmdbMovieHydration>(
        r#"
        SELECT
            e.export_artifact_id,
            e.tmdb_movie_id,
            e.original_title,
            e.popularity
        FROM tmdb_movie_exports e
        LEFT JOIN tmdb_movies m ON m.tmdb_movie_id = e.tmdb_movie_id
        WHERE e.export_artifact_id = ?
          AND (? = 1 OR e.adult = 0)
          AND m.tmdb_movie_id IS NULL
        ORDER BY e.popularity DESC, e.tmdb_movie_id ASC
        LIMIT ?
        "#,
    )
    .bind(export_artifact_id)
    .bind(if include_adult { 1 } else { 0 })
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn tmdb_movie_export_by_artifact_and_id(
    pool: &sqlx::SqlitePool,
    export_artifact_id: i64,
    tmdb_movie_id: i64,
) -> Result<Option<TmdbMovieExportEntry>> {
    let row = sqlx::query_as::<_, TmdbMovieExportEntry>(
        "SELECT * FROM tmdb_movie_exports WHERE export_artifact_id = ? AND tmdb_movie_id = ?",
    )
    .bind(export_artifact_id)
    .bind(tmdb_movie_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn mark_tmdb_movie_hydrated(
    pool: &sqlx::SqlitePool,
    export_artifact_id: i64,
    tmdb_movie_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE tmdb_movie_exports
        SET hydrate_status = 'completed',
            hydrate_attempts = hydrate_attempts + 1,
            last_error = NULL,
            hydrated_at = CURRENT_TIMESTAMP
        WHERE export_artifact_id = ? AND tmdb_movie_id = ?
        "#,
    )
    .bind(export_artifact_id)
    .bind(tmdb_movie_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_tmdb_movie_hydration_failed(
    pool: &sqlx::SqlitePool,
    export_artifact_id: i64,
    tmdb_movie_id: i64,
    error: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE tmdb_movie_exports
        SET hydrate_status = 'failed',
            hydrate_attempts = hydrate_attempts + 1,
            last_error = ?
        WHERE export_artifact_id = ? AND tmdb_movie_id = ?
        "#,
    )
    .bind(error)
    .bind(export_artifact_id)
    .bind(tmdb_movie_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_tmdb_movie(
    pool: &sqlx::SqlitePool,
    tmdb_movie_id: i64,
    imdb_id: Option<&str>,
    title: &str,
    original_title: Option<&str>,
    original_language: Option<&str>,
    overview: Option<&str>,
    release_date: Option<&str>,
    runtime_minutes: Option<i64>,
    status: Option<&str>,
    popularity: Option<f64>,
    vote_average: Option<f64>,
    vote_count: Option<i64>,
    raw_json: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO tmdb_movies (
            tmdb_movie_id, imdb_id, title, original_title, original_language, overview,
            release_date, runtime_minutes, status, popularity, vote_average, vote_count, raw_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(tmdb_movie_id) DO UPDATE SET
            imdb_id = excluded.imdb_id,
            title = excluded.title,
            original_title = excluded.original_title,
            original_language = excluded.original_language,
            overview = excluded.overview,
            release_date = excluded.release_date,
            runtime_minutes = excluded.runtime_minutes,
            status = excluded.status,
            popularity = excluded.popularity,
            vote_average = excluded.vote_average,
            vote_count = excluded.vote_count,
            raw_json = excluded.raw_json,
            hydrated_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(tmdb_movie_id)
    .bind(imdb_id)
    .bind(title)
    .bind(original_title)
    .bind(original_language)
    .bind(overview)
    .bind(release_date)
    .bind(runtime_minutes)
    .bind(status)
    .bind(popularity)
    .bind(vote_average)
    .bind(vote_count)
    .bind(raw_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_tmdb_person(
    pool: &sqlx::SqlitePool,
    tmdb_person_id: i64,
    name: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO tmdb_people (tmdb_person_id, name)
        VALUES (?, ?)
        ON CONFLICT(tmdb_person_id) DO UPDATE SET name = excluded.name
        "#,
    )
    .bind(tmdb_person_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn replace_tmdb_movie_credit(
    pool: &sqlx::SqlitePool,
    tmdb_movie_id: i64,
    tmdb_person_id: i64,
    credit_key: &str,
    cast_order: Option<i64>,
    credit_kind: &str,
    department: Option<&str>,
    job: Option<&str>,
    character_name: Option<&str>,
    raw_json: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO tmdb_movie_credits (
            tmdb_movie_id, tmdb_person_id, credit_key, cast_order, credit_kind, department, job, character_name, raw_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(tmdb_movie_id, credit_key) DO UPDATE SET
            tmdb_person_id = excluded.tmdb_person_id,
            cast_order = excluded.cast_order,
            credit_kind = excluded.credit_kind,
            department = excluded.department,
            job = excluded.job,
            character_name = excluded.character_name,
            raw_json = excluded.raw_json
        "#,
    )
    .bind(tmdb_movie_id)
    .bind(tmdb_person_id)
    .bind(credit_key)
    .bind(cast_order)
    .bind(credit_kind)
    .bind(department)
    .bind(job)
    .bind(character_name)
    .bind(raw_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_tmdb_movie_credits(pool: &sqlx::SqlitePool, tmdb_movie_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM tmdb_movie_credits WHERE tmdb_movie_id = ?")
        .bind(tmdb_movie_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn link_title_to_tmdb_movie(
    pool: &sqlx::SqlitePool,
    imdb_id: &str,
    tmdb_movie_id: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO title_tmdb_links (imdb_id, tmdb_movie_id, linked_via)
        VALUES (?, ?, 'tmdb_external_id')
        ON CONFLICT(imdb_id) DO UPDATE SET tmdb_movie_id = excluded.tmdb_movie_id, linked_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(imdb_id)
    .bind(tmdb_movie_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() >= 1)
}

pub async fn title_exists(pool: &sqlx::SqlitePool, imdb_id: &str) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT imdb_id FROM titles WHERE imdb_id = ?")
        .bind(imdb_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub async fn person_exists(pool: &sqlx::SqlitePool, imdb_name_id: &str) -> Result<bool> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT imdb_name_id FROM people WHERE imdb_name_id = ?")
            .bind(imdb_name_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

pub async fn clear_wikidata_import(pool: &sqlx::SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM wikidata_claims")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM title_wikidata_links")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM person_wikidata_links")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM wikidata_entities")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn upsert_wikidata_entity(
    pool: &sqlx::SqlitePool,
    wikidata_id: &str,
    label: Option<&str>,
    description: Option<&str>,
    entity_type: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO wikidata_entities (wikidata_id, label, description, entity_type)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(wikidata_id) DO UPDATE SET
            label = COALESCE(excluded.label, wikidata_entities.label),
            description = COALESCE(excluded.description, wikidata_entities.description),
            entity_type = COALESCE(excluded.entity_type, wikidata_entities.entity_type)
        "#,
    )
    .bind(wikidata_id)
    .bind(label)
    .bind(description)
    .bind(entity_type)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_wikidata_claims_for_subject(
    pool: &sqlx::SqlitePool,
    subject_wikidata_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM wikidata_claims WHERE subject_wikidata_id = ?")
        .bind(subject_wikidata_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_wikidata_claim(
    pool: &sqlx::SqlitePool,
    subject_wikidata_id: &str,
    property_id: &str,
    value_type: &str,
    value_text: Option<&str>,
    value_wikidata_id: Option<&str>,
    datatype: Option<&str>,
    rank_name: Option<&str>,
    ordinal: i64,
    raw_json: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO wikidata_claims (
            subject_wikidata_id, property_id, value_type, value_text, value_wikidata_id,
            datatype, rank_name, ordinal, raw_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(subject_wikidata_id, property_id, ordinal, value_type, value_text, value_wikidata_id)
        DO UPDATE SET
            datatype = excluded.datatype,
            rank_name = excluded.rank_name,
            raw_json = excluded.raw_json
        "#,
    )
    .bind(subject_wikidata_id)
    .bind(property_id)
    .bind(value_type)
    .bind(value_text)
    .bind(value_wikidata_id)
    .bind(datatype)
    .bind(rank_name)
    .bind(ordinal)
    .bind(raw_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_title_to_wikidata_entity(
    pool: &sqlx::SqlitePool,
    imdb_id: &str,
    wikidata_id: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO title_wikidata_links (imdb_id, wikidata_id, linked_via)
        VALUES (?, ?, 'wikidata_imdb_id')
        ON CONFLICT(imdb_id) DO UPDATE SET
            wikidata_id = excluded.wikidata_id,
            linked_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(imdb_id)
    .bind(wikidata_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() >= 1)
}

pub async fn link_person_to_wikidata_entity(
    pool: &sqlx::SqlitePool,
    imdb_name_id: &str,
    wikidata_id: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO person_wikidata_links (imdb_name_id, wikidata_id, linked_via)
        VALUES (?, ?, 'wikidata_imdb_id')
        ON CONFLICT(imdb_name_id) DO UPDATE SET
            wikidata_id = excluded.wikidata_id,
            linked_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(imdb_name_id)
    .bind(wikidata_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() >= 1)
}

pub async fn title_wikidata_links_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_local_id: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphWikidataLink>> {
    let rows = sqlx::query_as::<_, GraphWikidataLink>(
        r#"
        SELECT
            'title' AS entity_kind,
            t.imdb_id AS local_id,
            t.primary_title AS local_name,
            e.wikidata_id,
            e.label AS wikidata_label,
            e.description AS wikidata_description
        FROM title_wikidata_links l
        JOIN titles t ON t.imdb_id = l.imdb_id
        JOIN wikidata_entities e ON e.wikidata_id = l.wikidata_id
        WHERE (? IS NULL OR t.imdb_id > ?)
        ORDER BY t.imdb_id
        LIMIT ?
        "#,
    )
    .bind(last_local_id)
    .bind(last_local_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn person_wikidata_links_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_local_id: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphWikidataLink>> {
    let rows = sqlx::query_as::<_, GraphWikidataLink>(
        r#"
        SELECT
            'person' AS entity_kind,
            p.imdb_name_id AS local_id,
            p.primary_name AS local_name,
            e.wikidata_id,
            e.label AS wikidata_label,
            e.description AS wikidata_description
        FROM person_wikidata_links l
        JOIN people p ON p.imdb_name_id = l.imdb_name_id
        JOIN wikidata_entities e ON e.wikidata_id = l.wikidata_id
        WHERE (? IS NULL OR p.imdb_name_id > ?)
        ORDER BY p.imdb_name_id
        LIMIT ?
        "#,
    )
    .bind(last_local_id)
    .bind(last_local_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn title_wikidata_claims_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_claim_id: i64,
    limit: i64,
) -> Result<Vec<GraphWikidataClaim>> {
    let rows = sqlx::query_as::<_, GraphWikidataClaim>(
        r#"
        SELECT
            c.id AS claim_id,
            'title' AS entity_kind,
            l.imdb_id AS local_id,
            c.subject_wikidata_id,
            c.property_id,
            c.value_type,
            c.value_text,
            c.value_wikidata_id,
            v.label AS value_wikidata_label
        FROM wikidata_claims c
        JOIN title_wikidata_links l ON l.wikidata_id = c.subject_wikidata_id
        LEFT JOIN wikidata_entities v ON v.wikidata_id = c.value_wikidata_id
        WHERE c.id > ?
        ORDER BY c.id
        LIMIT ?
        "#,
    )
    .bind(last_claim_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn person_wikidata_claims_for_graph_page(
    pool: &sqlx::SqlitePool,
    last_claim_id: i64,
    limit: i64,
) -> Result<Vec<GraphWikidataClaim>> {
    let rows = sqlx::query_as::<_, GraphWikidataClaim>(
        r#"
        SELECT
            c.id AS claim_id,
            'person' AS entity_kind,
            l.imdb_name_id AS local_id,
            c.subject_wikidata_id,
            c.property_id,
            c.value_type,
            c.value_text,
            c.value_wikidata_id,
            v.label AS value_wikidata_label
        FROM wikidata_claims c
        JOIN person_wikidata_links l ON l.wikidata_id = c.subject_wikidata_id
        LEFT JOIN wikidata_entities v ON v.wikidata_id = c.value_wikidata_id
        WHERE c.id > ?
        ORDER BY c.id
        LIMIT ?
        "#,
    )
    .bind(last_claim_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
