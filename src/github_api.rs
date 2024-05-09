use reqwest::{header, Client, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tracing::info;

pub struct GithubApi {
    client: Client,
}

#[derive(Deserialize)]
struct Repositories {
    items: Vec<Repository>,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub stargazers_count: usize,
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

    pub async fn get_repositories_at_page(
        &self,
        min_star: usize,
        max_stars: usize,
        lang: &str,
        page: usize,
    ) -> Vec<Repository> {
        info!(
            "Getting repositories with stars between {} and {} and language {} at page {}",
            min_star, max_stars, lang, page
        );
        let q = format!(
            "stars:{}..{} language:{}",
            min_star,
            max_stars,
            lang.to_lowercase()
        );
        let params = [
            ("q", q.as_str()),
            ("order", "stars"),
            ("page", &page.to_string()),
            ("per_page", "100"),
        ];
        let repos: Repositories = self.get("search/repositories".to_string(), &params).await;
        repos.items
    }

    async fn get<T, K, V>(&self, name: String, params: &[(K, V)]) -> T
    where
        T: DeserializeOwned,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut url = "https://api.github.com/".to_string();
        url.push_str(&name);
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
