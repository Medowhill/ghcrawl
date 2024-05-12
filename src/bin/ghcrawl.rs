use futures::{pin_mut, stream::StreamExt};
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
    let repo_query = github_api::RepositoryQuery {
        min_stars: 1000,
        max_stars: 2000,
        lang: "c".to_string(),
    };
    let repos = api.get_repository_stream(&repo_query);
    pin_mut!(repos);
    while let Some(repo) = repos.next().await {
        println!("{}: {}", repo.full_name, repo.stargazers_count);
    }
    // let repos = api.get_repositories_between_stars(10000, 11000, "c").await;
    // for repo in repos {
    //     println!("{}: {}", repo.full_name, repo.stargazers_count);
    //     let langs = api.get_repository_languages(&repo.full_name).await;
    //     println!("{:?}", langs);
    //     let total_bytes = langs.values().sum::<usize>();
    //     if langs["C"] * 2 < total_bytes {
    //         continue;
    //     }
    //     let occurrences = api.get_occurrences(&repo.full_name, "c", "union").await;
    //     if occurrences.len() == 0 {
    //         continue;
    //     }
    //     println!("{:?}", occurrences);
    //     break;
    // }
}
