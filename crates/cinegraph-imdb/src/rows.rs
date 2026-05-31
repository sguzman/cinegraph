use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct NameBasicsRow {
    #[serde(rename = "nconst")]
    pub imdb_name_id: String,
    #[serde(rename = "primaryName")]
    pub primary_name: String,
    #[serde(rename = "birthYear")]
    pub birth_year: String,
    #[serde(rename = "deathYear")]
    pub death_year: String,
    #[serde(rename = "primaryProfession")]
    pub primary_professions: String,
    #[serde(rename = "knownForTitles")]
    pub known_for_titles: String,
}

#[derive(Debug, Deserialize)]
pub struct TitleBasicsRow {
    #[serde(rename = "tconst")]
    pub imdb_id: String,
    #[serde(rename = "titleType")]
    pub title_type: String,
    #[serde(rename = "primaryTitle")]
    pub primary_title: String,
    #[serde(rename = "originalTitle")]
    pub original_title: String,
    #[serde(rename = "isAdult")]
    pub is_adult: String,
    #[serde(rename = "startYear")]
    pub start_year: String,
    #[serde(rename = "endYear")]
    pub end_year: String,
    #[serde(rename = "runtimeMinutes")]
    pub runtime_minutes: String,
    pub genres: String,
}

#[derive(Debug, Deserialize)]
pub struct TitleRatingsRow {
    #[serde(rename = "tconst")]
    pub imdb_id: String,
    #[serde(rename = "averageRating")]
    pub average_rating: String,
    #[serde(rename = "numVotes")]
    pub num_votes: String,
}

#[derive(Debug, Deserialize)]
pub struct TitleAkasRow {
    #[serde(rename = "titleId")]
    pub imdb_id: String,
    pub ordering: String,
    pub title: String,
    pub region: String,
    pub language: String,
    pub types: String,
    pub attributes: String,
    #[serde(rename = "isOriginalTitle")]
    pub is_original_title: String,
}

#[derive(Debug, Deserialize)]
pub struct TitleCrewRow {
    #[serde(rename = "tconst")]
    pub imdb_id: String,
    pub directors: String,
    pub writers: String,
}

#[derive(Debug, Deserialize)]
pub struct TitlePrincipalsRow {
    #[serde(rename = "tconst")]
    pub imdb_id: String,
    pub ordering: String,
    #[serde(rename = "nconst")]
    pub imdb_name_id: String,
    pub category: String,
    pub job: String,
    pub characters: String,
}

#[derive(Debug, Deserialize)]
pub struct TitleEpisodeRow {
    #[serde(rename = "tconst")]
    pub imdb_id: String,
    #[serde(rename = "parentTconst")]
    pub parent_imdb_id: String,
    #[serde(rename = "seasonNumber")]
    pub season_number: String,
    #[serde(rename = "episodeNumber")]
    pub episode_number: String,
}
