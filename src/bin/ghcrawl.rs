use etrace::some_or;
use futures::{pin_mut, stream::StreamExt};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

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
        let occurrence_query = github_api::OccurrenceQuery {
            repo: &full_name,
            path: None,
            filename: None,
            lang: "c",
            token: "union",
        };
        let occurrences = api.get_occurrence_stream(occurrence_query);
        pin_mut!(occurrences);
        let occurrences = occurrences.collect::<Vec<_>>().await;
        if occurrences.is_empty() {
            continue;
        }
        let mut paths = vec![];
        for occurrence in &occurrences {
            let path = Path::new(&occurrence.path);
            let dir = some_or!(path.parent(), continue);
            let dir = some_or!(dir.as_os_str().to_str(), continue);
            let file = some_or!(path.file_name(), continue);
            let file = some_or!(file.to_str(), continue);
            let occurrence_query = github_api::OccurrenceQuery {
                repo: &full_name,
                path: Some(dir),
                filename: Some(file),
                lang: "c",
                token: "type",
            };
            let occurrences = api.get_occurrence_stream(occurrence_query);
            pin_mut!(occurrences);
            let occurrences = occurrences.collect::<Vec<_>>().await;
            if !occurrences.is_empty() {
                paths.push(&occurrence.path);
            }
        }
        if !paths.is_empty() {
            println!("{}: {}", full_name, stargazers_count);
            for path in paths {
                println!("  {}", path);
            }
        }
    }
}
