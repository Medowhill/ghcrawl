use futures::{pin_mut, stream::StreamExt};
use std::collections::HashMap;
use std::fs::OpenOptions;
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

    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(short, long)]
    input: Option<PathBuf>,

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

    if let Some(input) = args.input {
        let s = std::fs::read_to_string(input).unwrap();
        let mut repos: Vec<_> = s
            .split('\n')
            .filter_map(|s| {
                let mut cols = s.split(' ');
                let name = cols.next()?.to_string();
                let stars = cols.next()?.parse::<usize>().ok()?;
                let bytes = cols.next()?.parse::<usize>().ok()?;
                Some((name, stars, bytes))
            })
            .collect();
        repos.sort_by_key(|(_, _, bytes)| *bytes);
        for (name, stars, bytes) in &repos {
            let occurrence_query = github_api::OccurrenceQuery {
                repo: name,
                path: None,
                filename: None,
                lang: "c",
                token: "FILE",
            };
            let occurrences = api.get_occurrences(occurrence_query);
            pin_mut!(occurrences);
            let mut paths: HashMap<String, usize> = HashMap::new();
            while let Some(occurrence) = occurrences.next().await {
                *paths.entry(occurrence.path).or_default() += 1;
            }
            if !paths.is_empty() {
                let mut paths: Vec<_> = paths.into_iter().collect();
                paths.sort_by_key(|(_, count)| usize::MAX - *count);
                let mut s = String::new();
                for (p, n) in &paths {
                    use std::fmt::Write;
                    write!(s, "{}: {}, ", p, n).unwrap();
                }
                let s = format!("{} {} {}\n{}\n", name, stars, bytes, s);
                if let Some(output) = args.output.as_ref() {
                    append_to_file(output, s.as_str());
                } else {
                    print!("{}", s);
                }
            }
        }
    } else {
        let repo_query = github_api::RepositoryQuery {
            min_stars: 1000,
            max_stars: 128000,
            lang: "c",
        };
        let repos = api.get_repositories(repo_query);
        pin_mut!(repos);
        while let Some(repo) = repos.next().await {
            let github_api::Repository {
                full_name,
                stargazers_count,
            } = repo;

            let langs = api.get_repository_languages(&full_name).await;
            let total_bytes = langs.values().sum::<usize>();
            let c_bytes = langs["C"];
            if c_bytes * 2 < total_bytes {
                continue;
            }

            let s = format!("{} {} {}\n", full_name, stargazers_count, c_bytes);
            if let Some(output) = args.output.as_ref() {
                append_to_file(output, s.as_str());
            } else {
                print!("{}", s);
            }
        }
    }
}

fn append_to_file(path: &Path, content: &str) {
    use std::io::Write;
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    file.write_all(content.as_bytes()).unwrap();
}
