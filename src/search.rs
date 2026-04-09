use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::ProviderConnection,
};

#[derive(Debug, Clone, Copy)]
pub enum SearchHttpMethod {
    Get,
    Post,
}

pub struct SearchRequestSpec {
    pub method: SearchHttpMethod,
    pub url: String,
    pub body: Option<Value>,
}

pub fn build_search_request(provider: &ProviderConnection, payload: &Value, fallback_url: &str) -> AppResult<SearchRequestSpec> {
    let provider_id = provider.provider.as_str();
    let query = payload
        .get("query")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::BadRequest("search request requires query".into()))?;
    let search_type = payload
        .get("search_type")
        .or_else(|| payload.get("provider"))
        .and_then(|value| value.as_str())
        .unwrap_or("web");
    let max_results = payload
        .get("max_results")
        .or_else(|| payload.get("top_k"))
        .and_then(|value| value.as_u64())
        .unwrap_or(5)
        .clamp(1, 100);
    let country = payload.get("country").and_then(|value| value.as_str());
    let language = payload.get("language").and_then(|value| value.as_str());
    let domain_filter = payload
        .get("domain_filter")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(match provider_id {
        "serper-search" => {
            let endpoint = if search_type == "news" { "/news" } else { "/search" };
            let mut body = json!({
                "q": query,
                "num": max_results,
            });
            if let Some(country) = country {
                body["gl"] = Value::String(country.to_ascii_lowercase());
            }
            if let Some(language) = language {
                body["hl"] = Value::String(language.to_string());
            }
            let url = if search_url_is_overridden(provider, fallback_url) {
                fallback_url.to_string()
            } else {
                absolute_or_join(fallback_url, endpoint)
            };
            SearchRequestSpec {
                method: SearchHttpMethod::Post,
                url,
                body: Some(body),
            }
        }
        "brave-search" => {
            let endpoint = if search_type == "news" {
                "/res/v1/news/search"
            } else {
                "/res/v1/web/search"
            };
            let mut qp = vec![
                ("q", query.to_string()),
                ("count", max_results.to_string()),
            ];
            if let Some(country) = country {
                qp.push(("country", country.to_string()));
            }
            if let Some(language) = language {
                qp.push(("search_lang", language.to_string()));
            }
            let query_string = qp
                .into_iter()
                .map(|(k, v)| format!("{}={}", encode_query_component(k), encode_query_component(&v)))
                .collect::<Vec<_>>()
                .join("&");
            let base = if search_url_is_overridden(provider, fallback_url) {
                fallback_url.to_string()
            } else {
                absolute_or_join(fallback_url, endpoint)
            };
            SearchRequestSpec {
                method: SearchHttpMethod::Get,
                url: format!("{}?{}", base, query_string),
                body: None,
            }
        }
        "perplexity-search" => {
            let mut body = json!({
                "query": query,
                "max_results": max_results,
            });
            if let Some(country) = country {
                body["country"] = Value::String(country.to_string());
            }
            if let Some(language) = language {
                body["search_language_filter"] = Value::Array(vec![Value::String(language.to_string())]);
            }
            if !domain_filter.is_empty() {
                body["search_domain_filter"] = Value::Array(domain_filter.into_iter().map(Value::String).collect());
            }
            SearchRequestSpec {
                method: SearchHttpMethod::Post,
                url: fallback_url.to_string(),
                body: Some(body),
            }
        }
        "exa-search" => {
            let (includes, excludes) = split_domain_filters(&domain_filter);
            let mut body = json!({
                "query": query,
                "numResults": max_results,
                "type": "auto",
                "text": true,
                "highlights": true,
            });
            if !includes.is_empty() {
                body["includeDomains"] = Value::Array(includes.into_iter().map(Value::String).collect());
            }
            if !excludes.is_empty() {
                body["excludeDomains"] = Value::Array(excludes.into_iter().map(Value::String).collect());
            }
            if search_type == "news" {
                body["category"] = Value::String("news".to_string());
            }
            SearchRequestSpec {
                method: SearchHttpMethod::Post,
                url: fallback_url.to_string(),
                body: Some(body),
            }
        }
        "tavily-search" => {
            let (includes, excludes) = split_domain_filters(&domain_filter);
            let mut body = json!({
                "query": query,
                "max_results": max_results,
                "topic": if search_type == "news" { "news" } else { "general" },
            });
            if !includes.is_empty() {
                body["include_domains"] = Value::Array(includes.into_iter().map(Value::String).collect());
            }
            if !excludes.is_empty() {
                body["exclude_domains"] = Value::Array(excludes.into_iter().map(Value::String).collect());
            }
            if let Some(country) = country {
                body["country"] = Value::String(country.to_string());
            }
            SearchRequestSpec {
                method: SearchHttpMethod::Post,
                url: fallback_url.to_string(),
                body: Some(body),
            }
        }
        _ => SearchRequestSpec {
            method: SearchHttpMethod::Post,
            url: fallback_url.to_string(),
            body: Some(payload.clone()),
        },
    })
}

pub fn normalize_search_response(provider_id: &str, query: &str, search_type: &str, raw: Value) -> Value {
    let results = match provider_id {
        "serper-search" => normalize_serper(&raw, search_type),
        "brave-search" => normalize_brave(&raw, search_type),
        "perplexity-search" => normalize_perplexity(&raw),
        "exa-search" => normalize_exa(&raw),
        "tavily-search" => normalize_tavily(&raw),
        _ => raw.get("results").cloned().unwrap_or_else(|| Value::Array(vec![])),
    };

    let mut body = json!({
        "provider": provider_id,
        "query": query,
        "results": results,
        "answer": null,
        "usage": {
            "queries_used": 1,
            "search_cost_usd": search_cost(provider_id),
        },
        "metrics": {
            "response_time_ms": 0,
            "upstream_latency_ms": 0,
            "total_results_available": raw
                .get("searchParameters")
                .and_then(|value| value.get("totalResults"))
                .cloned()
                .unwrap_or(Value::Null),
        },
        "errors": []
    });
    if let Some(object) = body.as_object_mut() {
        if let Some(value) = raw.get("provider").cloned() {
            object.insert("upstream_provider".to_string(), value);
        }
        if let Some(value) = raw.get("auth_seen").cloned() {
            object.insert("auth_seen".to_string(), value);
        }
    }
    body
}

fn normalize_serper(raw: &Value, search_type: &str) -> Value {
    let items = if search_type == "news" {
        raw.get("news")
    } else {
        raw.get("organic")
    }
    .and_then(|value| value.as_array())
    .cloned()
    .unwrap_or_default();

    Value::Array(
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                json!({
                    "title": item.get("title").cloned().unwrap_or(Value::String(String::new())),
                    "url": item.get("link").cloned().unwrap_or(Value::String(String::new())),
                    "snippet": item
                        .get("snippet")
                        .cloned()
                        .or_else(|| item.get("description").cloned())
                        .unwrap_or(Value::String(String::new())),
                    "position": idx + 1,
                    "score": Value::Null,
                    "published_at": item.get("date").cloned().unwrap_or(Value::Null),
                    "favicon_url": Value::Null,
                    "content": Value::Null,
                    "metadata": Value::Null,
                    "citation": {"provider": "serper-search", "rank": idx + 1},
                })
            })
            .collect(),
    )
}

fn normalize_brave(raw: &Value, search_type: &str) -> Value {
    let container = if search_type == "news" {
        raw.get("news").or(Some(raw))
    } else {
        raw.get("web")
    };
    let items = container
        .and_then(|value| value.get("results"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Value::Array(
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                json!({
                    "title": item.get("title").cloned().unwrap_or(Value::String(String::new())),
                    "url": item.get("url").cloned().unwrap_or(Value::String(String::new())),
                    "snippet": item.get("description").cloned().unwrap_or(Value::String(String::new())),
                    "position": idx + 1,
                    "score": Value::Null,
                    "published_at": item
                        .get("page_age")
                        .cloned()
                        .or_else(|| item.get("age").cloned())
                        .unwrap_or(Value::Null),
                    "favicon_url": item
                        .get("meta_url")
                        .and_then(|value| value.get("favicon"))
                        .cloned()
                        .or_else(|| item.get("favicon").cloned())
                        .unwrap_or(Value::Null),
                    "content": Value::Null,
                    "metadata": Value::Null,
                    "citation": {"provider": "brave-search", "rank": idx + 1},
                })
            })
            .collect(),
    )
}

fn normalize_perplexity(raw: &Value) -> Value {
    let items = raw
        .get("results")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Value::Array(
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                json!({
                    "title": item.get("title").cloned().unwrap_or(Value::String(String::new())),
                    "url": item.get("url").cloned().unwrap_or(Value::String(String::new())),
                    "snippet": item.get("snippet").cloned().unwrap_or(Value::String(String::new())),
                    "position": idx + 1,
                    "score": Value::Null,
                    "published_at": item
                        .get("date")
                        .cloned()
                        .or_else(|| item.get("last_updated").cloned())
                        .unwrap_or(Value::Null),
                    "favicon_url": Value::Null,
                    "content": Value::Null,
                    "metadata": Value::Null,
                    "citation": {"provider": "perplexity-search", "rank": idx + 1},
                })
            })
            .collect(),
    )
}

fn normalize_exa(raw: &Value) -> Value {
    let items = raw
        .get("results")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Value::Array(
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                let snippet = item
                    .get("highlights")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .cloned()
                    .or_else(|| {
                        item.get("text")
                            .and_then(|value| value.as_str())
                            .map(|text| Value::String(text.chars().take(300).collect()))
                    })
                    .unwrap_or(Value::String(String::new()));
                json!({
                    "title": item.get("title").cloned().unwrap_or(Value::String(String::new())),
                    "url": item.get("url").cloned().unwrap_or(Value::String(String::new())),
                    "snippet": snippet,
                    "position": idx + 1,
                    "score": item.get("score").cloned().unwrap_or(Value::Null),
                    "published_at": item.get("publishedDate").cloned().unwrap_or(Value::Null),
                    "favicon_url": item.get("favicon").cloned().unwrap_or(Value::Null),
                    "content": item.get("text").map(|text| json!({
                        "format": "text",
                        "text": text,
                        "length": text.as_str().map(|value| value.len()).unwrap_or_default(),
                    })).unwrap_or(Value::Null),
                    "metadata": {
                        "author": item.get("author").cloned().unwrap_or(Value::Null),
                        "language": Value::Null,
                        "source_type": Value::Null,
                        "image_url": item.get("image").cloned().unwrap_or(Value::Null),
                    },
                    "citation": {"provider": "exa-search", "rank": idx + 1},
                })
            })
            .collect(),
    )
}

fn normalize_tavily(raw: &Value) -> Value {
    let items = raw
        .get("results")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    Value::Array(
        items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| {
                json!({
                    "title": item.get("title").cloned().unwrap_or(Value::String(String::new())),
                    "url": item.get("url").cloned().unwrap_or(Value::String(String::new())),
                    "snippet": item.get("content").cloned().unwrap_or(Value::String(String::new())),
                    "position": idx + 1,
                    "score": item.get("score").cloned().unwrap_or(Value::Null),
                    "published_at": item.get("published_date").cloned().unwrap_or(Value::Null),
                    "favicon_url": Value::Null,
                    "content": item.get("raw_content").map(|text| json!({
                        "format": "text",
                        "text": text,
                        "length": text.as_str().map(|value| value.len()).unwrap_or_default(),
                    })).unwrap_or(Value::Null),
                    "metadata": Value::Null,
                    "citation": {"provider": "tavily-search", "rank": idx + 1},
                })
            })
            .collect(),
    )
}

fn split_domain_filters(values: &[String]) -> (Vec<String>, Vec<String>) {
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for value in values {
        if let Some(stripped) = value.strip_prefix('-') {
            excludes.push(stripped.to_string());
        } else {
            includes.push(value.clone());
        }
    }
    (includes, excludes)
}

fn absolute_or_join(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}{}", base.trim_end_matches('/'), path)
    }
}

fn search_cost(provider_id: &str) -> f64 {
    match provider_id {
        "serper-search" => 0.001,
        "brave-search" => 0.005,
        "perplexity-search" => 0.005,
        "exa-search" => 0.007,
        "tavily-search" => 0.008,
        _ => 0.0,
    }
}


fn encode_query_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push_str("%20"),
            other => encoded.push_str(&format!("%{:02X}", other)),
        }
    }
    encoded
}

fn search_url_is_overridden(provider: &ProviderConnection, fallback_url: &str) -> bool {
    normalize_url(fallback_url) != normalize_url(&provider.base_url)
}

fn normalize_url(value: &str) -> &str {
    value.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn make_search_provider(provider_id: &str) -> crate::models::ProviderConnection {
        crate::models::ProviderConnection {
            id: String::new(),
            provider: provider_id.to_string(),
            base_url: "https://api.search.test".to_string(),
            api_key: Some("sk-key".to_string()),
            auth_type: "apikey".to_string(),
            auth_header: "bearer".to_string(),
            auth_prefix: None,
            extra_headers: BTreeMap::new(),
            endpoint_paths: BTreeMap::new(),
            stream_endpoint_paths: BTreeMap::new(),
            model_prefix: provider_id.to_string(),
            name: None,
            enabled: true,
            priority: 0,
            default_model: None,
            supported_endpoints: vec![],
            rate_limit_protection: false,
            last_error: None,
            last_error_at: None,
            last_error_type: None,
            last_error_source: None,
            rate_limited_until: None,
            circuit_open_until: None,
            last_used_at: None,
            backoff_level: 0,
            consecutive_use_count: 0,
            test_status: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            protocol_format: None,
        }
    }

    // --- build_search_request tests ---

    #[test]
    fn test_build_serper_request() {
        let provider = make_search_provider("serper-search");
        let payload = json!({"query": "rust async"});
        let spec = build_search_request(&provider, &payload, "https://api.search.test").unwrap();
        assert!(matches!(spec.method, SearchHttpMethod::Post));
        assert!(spec.url.contains("/search"));
        let body = spec.body.unwrap();
        assert_eq!(body["q"], "rust async");
        assert_eq!(body["num"], 5);
    }

    #[test]
    fn test_build_serper_news() {
        let provider = make_search_provider("serper-search");
        let payload = json!({"query": "headlines", "search_type": "news"});
        let spec = build_search_request(&provider, &payload, "https://api.search.test").unwrap();
        assert!(spec.url.contains("/news"));
        assert!(!spec.url.contains("/search"));
    }

    #[test]
    fn test_build_brave_request() {
        let provider = make_search_provider("brave-search");
        let payload = json!({"query": "test query", "max_results": 3});
        let spec = build_search_request(&provider, &payload, "https://api.search.test").unwrap();
        assert!(matches!(spec.method, SearchHttpMethod::Get));
        assert!(spec.url.contains("q="));
        assert!(spec.url.contains("count=3"));
        assert!(spec.body.is_none());
    }

    #[test]
    fn test_build_brave_country_language() {
        let provider = make_search_provider("brave-search");
        let payload = json!({"query": "q", "country": "US", "language": "en"});
        let spec = build_search_request(&provider, &payload, "https://api.search.test").unwrap();
        assert!(spec.url.contains("country=US"));
        assert!(spec.url.contains("search_lang=en"));
    }

    #[test]
    fn test_build_perplexity_request() {
        let provider = make_search_provider("perplexity-search");
        let payload = json!({"query": "meaning of life", "max_results": 10});
        let spec = build_search_request(&provider, &payload, "https://api.perplexity.test").unwrap();
        assert!(matches!(spec.method, SearchHttpMethod::Post));
        let body = spec.body.unwrap();
        assert_eq!(body["query"], "meaning of life");
        assert_eq!(body["max_results"], 10);
    }

    #[test]
    fn test_build_exa_request() {
        let provider = make_search_provider("exa-search");
        let payload = json!({"query": "neural networks"});
        let spec = build_search_request(&provider, &payload, "https://api.exa.test").unwrap();
        assert!(matches!(spec.method, SearchHttpMethod::Post));
        let body = spec.body.unwrap();
        assert_eq!(body["query"], "neural networks");
        assert_eq!(body["numResults"], 5);
        assert_eq!(body["type"], "auto");
        assert_eq!(body["text"], true);
        assert_eq!(body["highlights"], true);
    }

    #[test]
    fn test_build_exa_domain_filters() {
        let provider = make_search_provider("exa-search");
        let payload = json!({
            "query": "news",
            "domain_filter": ["example.com", "good.org", "-bad.net", "-spam.io"]
        });
        let spec = build_search_request(&provider, &payload, "https://api.exa.test").unwrap();
        let body = spec.body.unwrap();
        let includes = body["includeDomains"].as_array().unwrap();
        let excludes = body["excludeDomains"].as_array().unwrap();
        assert_eq!(includes.len(), 2);
        assert_eq!(includes[0], "example.com");
        assert_eq!(includes[1], "good.org");
        assert_eq!(excludes.len(), 2);
        assert_eq!(excludes[0], "bad.net");
        assert_eq!(excludes[1], "spam.io");
    }

    #[test]
    fn test_build_tavily_request() {
        let provider = make_search_provider("tavily-search");
        let payload = json!({"query": "climate change", "max_results": 7});
        let spec = build_search_request(&provider, &payload, "https://api.tavily.test").unwrap();
        assert!(matches!(spec.method, SearchHttpMethod::Post));
        let body = spec.body.unwrap();
        assert_eq!(body["query"], "climate change");
        assert_eq!(body["max_results"], 7);
        assert_eq!(body["topic"], "general");
    }

    #[test]
    fn test_build_tavily_news() {
        let provider = make_search_provider("tavily-search");
        let payload = json!({"query": "latest", "search_type": "news"});
        let spec = build_search_request(&provider, &payload, "https://api.tavily.test").unwrap();
        let body = spec.body.unwrap();
        assert_eq!(body["topic"], "news");
    }

    #[test]
    fn test_build_generic_request() {
        let provider = make_search_provider("unknown-provider");
        let payload = json!({"query": "hello", "custom_field": 42});
        let spec = build_search_request(&provider, &payload, "https://api.generic.test").unwrap();
        assert!(matches!(spec.method, SearchHttpMethod::Post));
        let body = spec.body.unwrap();
        assert_eq!(body, payload);
    }

    #[test]
    fn test_missing_query_error() {
        let provider = make_search_provider("serper-search");
        let payload = json!({"max_results": 5});
        let result = build_search_request(&provider, &payload, "https://api.search.test");
        assert!(result.is_err());
    }

    // --- normalize_search_response tests ---

    #[test]
    fn test_normalize_serper_response() {
        let raw = json!({
            "organic": [
                {"title": "Page A", "link": "https://a.test", "snippet": "Snippet A"},
                {"title": "Page B", "link": "https://b.test", "snippet": "Snippet B"}
            ]
        });
        let out = normalize_search_response("serper-search", "test", "web", raw);
        let results = out["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["title"], "Page A");
        assert_eq!(results[0]["url"], "https://a.test");
        assert_eq!(results[0]["snippet"], "Snippet A");
        assert_eq!(results[1]["position"], 2);
    }

    #[test]
    fn test_normalize_brave_response() {
        let raw = json!({
            "web": {
                "results": [
                    {"title": "Brave 1", "url": "https://b1.test", "description": "Desc 1"},
                    {"title": "Brave 2", "url": "https://b2.test", "description": "Desc 2"}
                ]
            }
        });
        let out = normalize_search_response("brave-search", "q", "web", raw);
        let results = out["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["url"], "https://b1.test");
        assert_eq!(results[0]["snippet"], "Desc 1");
        assert_eq!(results[1]["title"], "Brave 2");
    }

    #[test]
    fn test_normalize_perplexity_response() {
        let raw = json!({
            "results": [
                {"title": "Perp 1", "url": "https://p1.test", "snippet": "S1"}
            ]
        });
        let out = normalize_search_response("perplexity-search", "q", "web", raw);
        let results = out["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "Perp 1");
        assert_eq!(results[0]["url"], "https://p1.test");
        assert_eq!(results[0]["snippet"], "S1");
    }

    #[test]
    fn test_normalize_exa_response() {
        let raw = json!({
            "results": [
                {
                    "title": "Exa Page",
                    "url": "https://exa.test",
                    "highlights": ["First highlight", "Second highlight"],
                    "text": "Full text content"
                }
            ]
        });
        let out = normalize_search_response("exa-search", "q", "web", raw);
        let results = out["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["snippet"], "First highlight");
        assert_eq!(results[0]["title"], "Exa Page");
    }

    #[test]
    fn test_normalize_tavily_response() {
        let raw = json!({
            "results": [
                {"title": "Tavily 1", "url": "https://t1.test", "content": "Tavily content"}
            ]
        });
        let out = normalize_search_response("tavily-search", "q", "web", raw);
        let results = out["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["snippet"], "Tavily content");
        assert_eq!(results[0]["url"], "https://t1.test");
    }

    #[test]
    fn test_normalize_response_has_metadata() {
        let raw = json!({"results": []});
        let out = normalize_search_response("serper-search", "q", "web", raw);
        assert!(out.get("provider").is_some());
        assert!(out.get("query").is_some());
        assert!(out.get("results").is_some());
        assert!(out.get("usage").is_some());
        assert!(out.get("metrics").is_some());
        assert_eq!(out["provider"], "serper-search");
        assert_eq!(out["query"], "q");
        assert_eq!(out["usage"]["queries_used"], 1);
    }
}