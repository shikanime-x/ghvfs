mod cli;
mod db;
mod doc;
mod gh;
mod meta;
mod model;
mod ops;
mod watch;

use clap::Parser;
use cli::{Cli, Commands};

async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Sync {
            repo,
            root,
            project,
        } => {
            let pool = db::open(&root).await?;
            ops::sync(&repo, &root, &pool, &project).await
        }
        Commands::Apply {
            root,
            commit,
            force,
        } => {
            let pool = db::open(&root).await?;
            ops::apply(&root, &pool, commit, force).await
        }
        Commands::Pull {
            repo,
            root,
            project,
        } => {
            let pool = db::open(&root).await?;
            ops::pull(&repo, &root, &pool, "1970-01-01T00:00:00Z", &project)
                .await
                .map(|_| ())
        }
        Commands::Unsync { repo, root } => {
            let pool = db::open(&root).await?;
            ops::unsync(&repo, &root, &pool).await
        }
        Commands::Watch {
            root,
            commit,
            force,
            pull,
            repo,
            interval,
        } => {
            if pull {
                watch::watch_pull(&repo, &root, interval).await
            } else {
                watch::watch(root, commit, force).await
            }
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("ghvfs: {e}");
        std::process::exit(1);
    }
}
