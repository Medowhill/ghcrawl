use futures::stream::Stream;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use reqwest::{header, Client, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tracing::info;

const PER_PAGE: usize = 100;

pub struct GithubApi {
    client: Client,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct FileContent {
    pub encoding: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct Occurrence {
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub stargazers_count: usize,
}

#[derive(Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
}

#[derive(Clone, Copy)]
pub struct OccurrenceQuery<'a> {
    pub repo: &'a str,
    pub path: Option<&'a str>,
    pub filename: Option<&'a str>,
    pub lang: &'static str,
    pub token: &'static str,
}

#[derive(Clone, Copy)]
pub struct RepositoryQuery {
    pub min_stars: usize,
    pub max_stars: usize,
    pub lang: &'static str,
}

enum ApiResult<T> {
    Success(T),
    RateLimit(u64),
    SecondaryLimit,
}

impl GithubApi {
    #[inline]
    pub fn new(token: String) -> Self {
        let mut headers = header::HeaderMap::new();
        let v = header::HeaderValue::from_static("ghcrawl");
        headers.insert("User-Agent", v);
        let v = header::HeaderValue::from_str(&format!("token {}", token)).unwrap();
        headers.insert("Authorization", v);
        let client = Client::builder().default_headers(headers).build().unwrap();
        Self { client }
    }

    pub async fn get_repository_languages(&self, repo: &str) -> HashMap<String, usize> {
        let path = format!("repos/{}/languages", repo);
        self.get::<_, &str, &str>(path, &[]).await
    }

    pub async fn get_file_content(&self, repo: &str, path: &str) -> FileContent {
        let path = format!("repos/{}/contents/{}", repo, path);
        self.get::<_, &str, &str>(path, &[]).await
    }

    pub fn get_occurrences<'a>(
        &'a self,
        q: OccurrenceQuery<'a>,
    ) -> impl Stream<Item = Occurrence> + 'a {
        page_stream(q, |q, page| self.get_occurrences_at_page(q, page))
    }

    pub async fn get_occurrences_at_page(
        &self,
        q: OccurrenceQuery<'_>,
        page: usize,
    ) -> Page<Occurrence> {
        let path = if let Some(path) = q.path {
            format!(" path:{}", path)
        } else {
            "".to_string()
        };
        let filename = if let Some(filename) = q.filename {
            format!(" filename:{}", filename)
        } else {
            "".to_string()
        };
        let q = format!(
            "{} repo:{} language:{}{}{}",
            q.token, q.repo, q.lang, path, filename
        );
        let params = [
            ("q", q.as_str()),
            ("page", &page.to_string()),
            ("per_page", &PER_PAGE.to_string()),
        ];
        self.get("search/code".to_string(), &params).await
    }

    pub fn get_repositories(
        &self,
        mut q: RepositoryQuery,
    ) -> impl Stream<Item = Repository> + '_ {
        let min_stars = q.min_stars;
        let max_stars = q.max_stars;
        futures::stream::unfold(max_stars, move |max_stars| async move {
            q.max_stars = max_stars;
            q.min_stars = max_stars / 2;
            if q.min_stars < min_stars {
                None
            } else {
                let repos = self.get_repositories_with_stars(q);
                Some((repos, q.min_stars))
            }
        })
        .flatten()
    }

    pub fn get_repositories_with_stars(
        &self,
        q: RepositoryQuery,
    ) -> impl Stream<Item = Repository> + '_ {
        page_stream(q, |q, page| self.get_repositories_at_page(q, page))
    }

    pub async fn get_repositories_at_page(
        &self,
        q: RepositoryQuery,
        page: usize,
    ) -> Page<Repository> {
        let q = format!(
            "stars:{}..{} language:{}",
            q.min_stars,
            q.max_stars,
            q.lang.to_lowercase()
        );
        let params = [
            ("q", q.as_str()),
            ("order", "stars"),
            ("page", &page.to_string()),
            ("per_page", &PER_PAGE.to_string()),
        ];
        self.get("search/repositories".to_string(), &params).await
    }

    async fn get<T, K, V>(&self, path: String, params: &[(K, V)]) -> T
    where
        T: DeserializeOwned,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut url = "https://api.github.com/".to_string();
        url.push_str(&path);
        url.push('?');
        for (i, (k, v)) in params.iter().enumerate() {
            if i > 0 {
                url.push('&');
            }
            url.push_str(k.as_ref());
            url.push('=');
            url.push_str(v.as_ref());
        }
        self.get_from_url(&url).await
    }

    async fn get_from_url<T: DeserializeOwned>(&self, url: &str) -> T {
        loop {
            let wait = match self.try_get_from_url(url).await {
                ApiResult::Success(t) => break t,
                ApiResult::RateLimit(reset) => {
                    info!("Rate limit exceeded, waiting for {} seconds", reset);
                    reset
                }
                ApiResult::SecondaryLimit => {
                    info!("Secondary limit exceeded, waiting for 60 seconds");
                    60
                }
            };
            tokio::time::sleep(tokio::time::Duration::from_secs(wait)).await;
        }
    }

    async fn try_get_from_url<T: DeserializeOwned>(&self, url: &str) -> ApiResult<T> {
        info!("GET {}", url);
        let response = self.client.get(url).send().await.unwrap();
        if response.status().is_success() {
            ApiResult::Success(response.json().await.unwrap())
        } else {
            let reset = get_reset(&response);
            let text = response.text().await.unwrap();
            if text.contains("secondary") {
                ApiResult::SecondaryLimit
            } else {
                ApiResult::RateLimit(reset)
            }
        }
    }
}

#[inline]
fn page_stream<'a, Q, R, Fut, F>(query: Q, f: F) -> impl Stream<Item = R> + 'a
where
    Q: Copy + 'a,
    R: 'a,
    Fut: Future<Output = Page<R>> + 'a,
    F: Copy + Fn(Q, usize) -> Fut + 'a,
{
    futures::stream::unfold(Some(1), move |page| async move {
        let page = page?;
        let vs = f(query, page).await;
        let next = if page == 10 || vs.items.len() < PER_PAGE {
            None
        } else {
            Some(page + 1)
        };
        Some((vs.items, next))
    })
    .map(futures::stream::iter)
    .flatten()
}

fn get_reset(res: &Response) -> u64 {
    res.headers()
        .get("X-RateLimit-Reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| {
            v - SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        })
        .unwrap_or(0)
        + 1
}
