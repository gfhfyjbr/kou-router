//! Per-account proxy configuration and shared reqwest client caching.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use reqwest::{Client, Proxy};

use crate::error::{AppError, AppResult};

/// Cache of direct and proxied HTTP clients keyed by `proxy_url`.
#[derive(Clone)]
pub struct ProxyClientCache {
    direct: Result<Client, String>,
    cache: Arc<Mutex<HashMap<String, Client>>>,
}

impl ProxyClientCache {
    pub fn new() -> Self {
        Self {
            direct: build_direct_client(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn direct(&self) -> AppResult<Client> {
        self.direct
            .clone()
            .map_err(|err| AppError::Upstream(format!("failed to build no-proxy client: {err}")))
    }

    /// Returns a cached client configured to send all traffic through `proxy_url`.
    pub fn for_proxy(&self, proxy_url: &str) -> AppResult<Client> {
        parse_and_validate(proxy_url)?;
        let trimmed = proxy_url.trim();
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| AppError::Upstream("proxy cache poisoned".to_owned()))?;
        if let Some(client) = cache.get(trimmed) {
            return Ok(client.clone());
        }
        let client = build_client(trimmed)?;
        cache.insert(trimmed.to_owned(), client.clone());
        Ok(client)
    }

    pub fn for_optional(&self, proxy_url: Option<&str>) -> AppResult<Client> {
        match proxy_url {
            Some(proxy_url) => self.for_proxy(proxy_url),
            None => self.direct(),
        }
    }

    #[cfg(test)]
    fn len(&self) -> AppResult<usize> {
        self.cache
            .lock()
            .map(|cache| cache.len())
            .map_err(|_| AppError::Upstream("proxy cache poisoned".to_owned()))
    }
}

fn build_direct_client() -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .build()
        .map_err(|err| err.to_string())
}

pub fn parse_and_validate(proxy_url: &str) -> AppResult<()> {
    let trimmed = proxy_url.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("proxy_url is empty".to_owned()));
    }
    Proxy::all(trimmed)
        .map(|_| ())
        .map_err(|err| AppError::BadRequest(format!("invalid proxy_url '{trimmed}': {err}")))
}

fn build_client(proxy_url: &str) -> AppResult<Client> {
    let proxy = Proxy::all(proxy_url)
        .map_err(|err| AppError::BadRequest(format!("invalid proxy_url '{proxy_url}': {err}")))?;
    Client::builder()
        .no_proxy()
        .proxy(proxy)
        .build()
        .map_err(|err| AppError::Upstream(format!("failed to build proxy client: {err}")))
}

#[cfg(test)]
mod tests {
    use super::{ProxyClientCache, parse_and_validate};
    use crate::error::AppError;

    #[test]
    fn valid_http_url_builds() {
        ProxyClientCache::new()
            .for_proxy("http://127.0.0.1:8080")
            .unwrap();
    }

    #[test]
    fn valid_socks5_url_builds() {
        ProxyClientCache::new()
            .for_proxy("socks5://127.0.0.1:1080")
            .unwrap();
    }

    #[test]
    fn valid_socks5h_url_builds() {
        ProxyClientCache::new()
            .for_proxy("socks5h://127.0.0.1:1080")
            .unwrap();
    }

    #[test]
    fn empty_url_is_bad_request() {
        assert!(matches!(
            parse_and_validate("  "),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn garbage_url_is_bad_request() {
        assert!(matches!(
            parse_and_validate("not a url"),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn cache_returns_same_client_for_same_url() {
        let cache = ProxyClientCache::new();
        cache.for_proxy("socks5h://127.0.0.1:1080").unwrap();
        cache.for_proxy("socks5h://127.0.0.1:1080").unwrap();
        assert_eq!(cache.len().unwrap(), 1);
    }

    #[test]
    fn cache_separates_distinct_urls() {
        let cache = ProxyClientCache::new();
        cache.for_proxy("socks5h://127.0.0.1:1080").unwrap();
        cache.for_proxy("socks5h://127.0.0.1:1081").unwrap();
        assert_eq!(cache.len().unwrap(), 2);
    }
}
