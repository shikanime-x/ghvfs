use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "ghvfs",
    version,
    about = "GitHub issues as a local filesystem tree"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Materialize a repository into a local tree and snapshot its state
    Sync {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        root: PathBuf,
    },
    /// Apply local title and body edits back to GitHub
    Apply {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        commit: bool,
        #[arg(long)]
        force: bool,
    },
    /// Apply edits automatically after a document is saved
    Watch {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        commit: bool,
        #[arg(long)]
        force: bool,
    },
}
