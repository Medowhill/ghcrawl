use futures::stream::Stream;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use reqwest::{header, Client, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tracing::info;

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
struct Collection<T> {
    items: Vec<T>,
}

pub struct RepositoryStream<'a> {
    stream: Box<dyn Stream<Item = Repository> + 'a>,
    _phantom: std::marker::PhantomData<&'a GithubApi>,
}

impl Stream for RepositoryStream<'_> {
    type Item = Repository;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe {
            let stream = Pin::new_unchecked(self.get_mut().stream.as_mut());
            stream.poll_next(cx)
        }
    }
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

    pub async fn get_occurrences(&self, repo: &str, lang: &str, tok: &str) -> Vec<Occurrence> {
        let q = format!("{} repo:{} language:{}", tok, repo, lang);
        let params = [("q", q.as_str()), ("per_page", "100")];
        let occurrences: Collection<Occurrence> =
            self.get("search/code".to_string(), &params).await;
        occurrences.items
    }

    pub async fn get_repository_languages(&self, repo: &str) -> HashMap<String, usize> {
        let path = format!("repos/{}/languages", repo);
        self.get::<_, &str, &str>(path, &[]).await
    }

    pub fn repository_stream<'a>(
        &'a self,
        min_stars: usize,
        max_stars: usize,
        lang: &'a str,
    ) -> RepositoryStream<'a> {
        let repos = futures::stream::unfold(Some(1usize), move |page| async move {
            let page = page?;
            let repos = self
                .get_repositories_at_page(min_stars, max_stars, lang, page)
                .await;
            let next = if repos.len() < 100 {
                None
            } else {
                Some(page + 1)
            };
            Some((repos, next))
        })
        .map(futures::stream::iter)
        .flatten();
        RepositoryStream {
            stream: Box::new(repos),
            _phantom: std::marker::PhantomData,
        }
    }

    pub async fn get_repositories_at_page(
        &self,
        min_stars: usize,
        max_stars: usize,
        lang: &str,
        page: usize,
    ) -> Vec<Repository> {
        info!(
            "Getting repositories with stars between {} and {} and language {} at page {}",
            min_stars, max_stars, lang, page
        );
        let q = format!(
            "stars:{}..{} language:{}",
            min_stars,
            max_stars,
            lang.to_lowercase()
        );
        let params = [
            ("q", q.as_str()),
            ("order", "stars"),
            ("page", &page.to_string()),
            ("per_page", "100"),
        ];
        let repos: Collection<Repository> =
            self.get("search/repositories".to_string(), &params).await;
        repos.items
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

fn get_reset(res: &Response) -> u64 {
    res.headers()
        .get("X-RateLimit-Reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}
