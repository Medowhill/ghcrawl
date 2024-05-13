use futures::{pin_mut, stream::StreamExt};
use std::{collections::HashSet, fs::File, path::PathBuf};

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
        max_stars: 128000,
        lang: "c",
    };
    let repos = api.get_repository_stream(repo_query);
    pin_mut!(repos);
    while let Some(repo) = repos.next().await {
        let github_api::Repository {
            full_name,
            stargazers_count,
        } = repo;

        let langs = api.get_repository_languages(&full_name).await;
        let total_bytes = langs.values().sum::<usize>();
        if langs["C"] * 2 < total_bytes {
            continue;
        }

        let mut occurrence_query = github_api::OccurrenceQuery {
            repo: &full_name,
            path: None,
            filename: None,
            lang: "c",
            token: "union",
        };
        let occurrences = api.get_occurrence_stream(occurrence_query);
        pin_mut!(occurrences);
        let mut occurrences = occurrences.collect::<Vec<_>>().await;
        if occurrences.is_empty() {
            continue;
        }

        occurrence_query.token = "type";
        let type_occurrences = api.get_occurrence_stream(occurrence_query);
        pin_mut!(type_occurrences);
        let type_occurrences = type_occurrences.collect::<HashSet<_>>().await;

        occurrences.retain(|occurrence| type_occurrences.contains(occurrence));
        if occurrences.is_empty() {
            continue;
        }

        println!("{}: {}", full_name, stargazers_count);
        for occurrence in occurrences {
            println!("  {}", occurrence.path);
        }
    }
}
