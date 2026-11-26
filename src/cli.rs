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
        /// Project boards to read fields from, as <owner>/<number>
        #[arg(long)]
        project: Vec<String>,
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
    /// Pull changed issues and PRs from GitHub into the tree once
    Pull {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        root: PathBuf,
        /// Project boards to read fields from, as <owner>/<number>
        #[arg(long)]
        project: Vec<String>,
    },
    /// Remove a synced repository from the tree and the snapshot database
    Unsync {
        #[arg(long)]
        repo: String,
        #[arg(long)]
        root: PathBuf,
    },
    /// Apply edits automatically after a document is saved
    Watch {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        commit: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        pull: bool,
        #[arg(long)]
        repo: Vec<String>,
        #[arg(long, default_value_t = 30)]
        interval: u64,
    },
}
