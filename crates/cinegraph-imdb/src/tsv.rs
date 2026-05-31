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
        .from_reader(MultiGzDecoder::new(file));
    Ok(reader.into_deserialize())
}

#[cfg(test)]
mod tests {
    use super::imdb_null;

    #[test]
    fn imdb_null_maps_backslash_n_to_none() {
        assert_eq!(imdb_null(r"\N"), None);
        assert_eq!(imdb_null("value"), Some("value"));
    }
}
