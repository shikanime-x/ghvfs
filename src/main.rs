mod cli;
mod db;
mod doc;
mod gh;
mod model;
mod ops;
mod watch;

use clap::Parser;
use cli::{Cli, Commands};

async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sync { repo, root } => {
            let pool = db::open(&root).await?;
            ops::sync(&repo, &root, &pool).await
        }
        Commands::Apply {
            root,
            commit,
            force,
        } => {
            let pool = db::open(&root).await?;
            ops::apply(&root, &pool, commit, force).await
        }
        Commands::Watch {
            root,
            commit,
            force,
        } => watch::watch(root, commit, force).await,
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ghvfs: {e}");
        std::process::exit(1);
    }
}
