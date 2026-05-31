use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "cinegraph")]
pub struct Cli {
    #[arg(
        long,
        env = "CINEGRAPH_CONFIG",
        default_value = "config/cinegraph.example.toml"
    )]
    pub config: PathBuf,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init,
    Index {
        #[command(subcommand)]
        command: IndexCommands,
    },
    Fetch {
        #[command(subcommand)]
        command: FetchCommands,
    },
    Import {
        #[command(subcommand)]
        command: ImportCommands,
    },
    Stats,
    Search {
        #[command(subcommand)]
        command: SearchCommands,
    },
    Lookup {
        #[command(subcommand)]
        command: LookupCommands,
    },
    Doctor,
}

#[derive(Debug, Subcommand)]
pub enum FetchCommands {
    Imdb,
}

#[derive(Debug, Subcommand)]
pub enum ImportCommands {
    Imdb,
}

#[derive(Debug, Subcommand)]
pub enum IndexCommands {
    Search,
}

#[derive(Debug, Subcommand)]
pub enum SearchCommands {
    Title {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    Person {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum LookupCommands {
    Title { query: String },
    Person { query: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_search_command() {
        let cli = Cli::try_parse_from(["cinegraph", "index", "search"]).expect("parse");
        assert!(matches!(
            cli.command,
            Commands::Index {
                command: IndexCommands::Search
            }
        ));
    }

    #[test]
    fn parses_search_title_command_with_limit() {
        let cli = Cli::try_parse_from([
            "cinegraph",
            "search",
            "title",
            "kurosawa samurai",
            "--limit",
            "5",
        ])
        .expect("parse");
        assert!(matches!(
            cli.command,
            Commands::Search {
                command: SearchCommands::Title { limit: 5, .. }
            }
        ));
    }
}
