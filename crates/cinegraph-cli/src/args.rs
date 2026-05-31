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
    Fetch {
        #[command(subcommand)]
        command: FetchCommands,
    },
    Import {
        #[command(subcommand)]
        command: ImportCommands,
    },
    Stats,
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
pub enum LookupCommands {
    Title { query: String },
    Person { query: String },
}
