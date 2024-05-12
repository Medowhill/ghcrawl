use futures::stream::Stream;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::future::Future;

use reqwest::{header, Client, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tracing::info;

const PER_PAGE: usize = 100;

pub struct GithubApi {
    client: Client,
}

#[derive(Debug, Deserialize)]
pub struct Occurrence {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub stargazers_count: usize,
}

#[derive(Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
}

pub struct OccurrenceQuery {
    pub repo: String,
    pub lang: String,
    pub token: String,
}

pub struct RepositoryQuery {
    pub min_stars: usize,
    pub max_stars: usize,
    pub lang: String,
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

    pub fn get_occurrence_stream<'a>(
        &'a self,
        q: &'a OccurrenceQuery,
    ) -> impl Stream<Item = Occurrence> + 'a {
        page_stream(q, |q, page| self.get_occurrences_at_page(q, page))
    }

    pub async fn get_occurrences_at_page(
        &self,
        q: &OccurrenceQuery,
        page: usize,
    ) -> Page<Occurrence> {
        let q = format!("{} repo:{} language:{}", q.token, q.repo, q.lang);
        let params = [
            ("q", q.as_str()),
            ("page", &page.to_string()),
            ("per_page", &PER_PAGE.to_string()),
        ];
        self.get("search/code".to_string(), &params).await
    }

    pub fn get_repository_stream<'a>(
        &'a self,
        q: &'a RepositoryQuery,
    ) -> impl Stream<Item = Repository> + 'a {
        page_stream(q, |q, page| self.get_repositories_at_page(q, page))
    }

    pub async fn get_repositories_at_page(
        &self,
        q: &RepositoryQuery,
        page: usize,
    ) -> Page<Repository> {
        info!(
            "Getting repositories with stars between {} and {} and language {} at page {}",
            q.min_stars, q.max_stars, q.lang, page
        );
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
fn page_stream<'a, Q, R, Fut, F>(query: &'a Q, f: F) -> impl Stream<Item = R> + 'a
where
    Q: 'a,
    R: 'a,
    Fut: Future<Output = Page<R>> + 'a,
    F: Copy + Fn(&'a Q, usize) -> Fut + 'a,
{
    futures::stream::unfold(Some(1usize), move |page| async move {
        let page = page?;
        let vs = f(query, page).await;
        let next = if vs.items.len() < PER_PAGE {
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
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}
