use std::{fs::File, path::PathBuf};

use clap::Parser;

use ghcrawl::*;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    log_file: Option<PathBuf>,

    token_file: PathBuf,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if let Some(log_file) = args.log_file {
        let log_file = File::create(log_file).unwrap();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(log_file)
            .init();
    }

    let token = std::fs::read_to_string(args.token_file).unwrap();
    let api = github_api::GithubApi::new(token.trim().to_string());
    let repos = api.get_repositories_at_page(10000, 20000, "c", 1).await;
    for repo in repos {
        println!("{}: {}", repo.full_name, repo.stargazers_count);
    }
}
