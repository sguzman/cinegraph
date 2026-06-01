use cinegraph_core::Result;
use csv::ReaderBuilder;
use flate2::read::MultiGzDecoder;
use serde::de::DeserializeOwned;
use std::{fs::File, path::Path};

pub fn imdb_null(value: &str) -> Option<&str> {
    if value == r"\N" { None } else { Some(value) }
}

pub fn read_gzip_tsv<T: DeserializeOwned>(
    path: &Path,
) -> Result<csv::DeserializeRecordsIntoIter<MultiGzDecoder<File>, T>> {
    let file = File::open(path)?;
    let reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .quoting(false)
        .from_reader(MultiGzDecoder::new(file));
    Ok(reader.into_deserialize())
}

#[cfg(test)]
mod tests {
    use super::imdb_null;
    use crate::rows::TitleBasicsRow;
    use csv::ReaderBuilder;

    #[test]
    fn imdb_null_maps_backslash_n_to_none() {
        assert_eq!(imdb_null(r"\N"), None);
        assert_eq!(imdb_null("value"), Some("value"));
    }

    #[test]
    fn tsv_reader_treats_literal_quotes_as_plain_text() {
        let data = concat!(
            "tconst\ttitleType\tprimaryTitle\toriginalTitle\tisAdult\tstartYear\tendYear\truntimeMinutes\tgenres\n",
            "tt10233364\ttvEpisode\t\"Rolling in the Deep Dish\t\"Rolling in the Deep Dish\t0\t2019\t\\N\t\\N\tReality-TV\n",
        );

        let mut reader = ReaderBuilder::new()
            .delimiter(b'\t')
            .has_headers(true)
            .quoting(false)
            .from_reader(data.as_bytes());
        let row = reader
            .deserialize::<TitleBasicsRow>()
            .next()
            .expect("row")
            .expect("deserialize");

        assert_eq!(row.primary_title, "\"Rolling in the Deep Dish");
        assert_eq!(row.original_title, "\"Rolling in the Deep Dish");
    }
}
