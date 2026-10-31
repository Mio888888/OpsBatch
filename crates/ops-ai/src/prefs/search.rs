//! Web search (`WebSearchConfig`, the provider presets and `executeWebSearchProvider` of Netcatty).

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub struct SearchPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub default_api_host: &'static str,
    pub requires_api_key: bool,
}

pub const SEARCH_PRESETS: &[SearchPreset] = &[
    SearchPreset {
        id: "tavily",
        name: "Tavily",
        default_api_host: "https://api.tavily.com",
        requires_api_key: true,
    },
    SearchPreset {
        id: "exa",
        name: "Exa",
        default_api_host: "https://api.exa.ai",
        requires_api_key: true,
    },
    SearchPreset {
        id: "bocha",
        name: "Bocha",
        default_api_host: "https://api.bochaai.com",
        requires_api_key: true,
    },
    SearchPreset {
        id: "zhipu",
        name: "Zhipu",
        default_api_host: "https://open.bigmodel.cn/api/paas/v4",
        requires_api_key: true,
    },
    SearchPreset {
        id: "searxng",
        name: "SearXNG",
        default_api_host: "",
        requires_api_key: false,
    },
];

pub fn search_preset(id: &str) -> &'static SearchPreset {
    SEARCH_PRESETS
        .iter()
        .find(|p| p.id == id)
        .unwrap_or(&SEARCH_PRESETS[0])
}

pub const DEFAULT_MAX_RESULTS: u32 = 5;
pub const MAX_RESULTS_LIMIT: u32 = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSearchConfig {
    pub provider_id: String,
    pub api_key: Option<String>,
    /// Custom endpoint; required for SearXNG.
    pub api_host: Option<String>,
    pub enabled: bool,
    pub max_results: Option<u32>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider_id: "tavily".into(),
            api_key: None,
            api_host: None,
            enabled: false,
            max_results: Some(DEFAULT_MAX_RESULTS),
        }
    }
}

impl WebSearchConfig {
    pub fn normalize(&mut self) {
        if !SEARCH_PRESETS.iter().any(|p| p.id == self.provider_id) {
            self.provider_id = "tavily".into();
        }
        self.max_results = Some(
            self.max_results
                .unwrap_or(DEFAULT_MAX_RESULTS)
                .clamp(1, MAX_RESULTS_LIMIT),
        );
        self.api_host = self
            .api_host
            .take()
            .map(|h| h.trim().to_string())
            .filter(|h| !h.is_empty());
    }

    pub fn preset(&self) -> &'static SearchPreset {
        search_preset(&self.provider_id)
    }

    /// Switching the provider resets the key and the host to the new provider's defaults.
    pub fn switch_provider(&mut self, provider_id: &str) {
        let preset = search_preset(provider_id);
        self.provider_id = preset.id.to_string();
        self.api_key = None;
        self.api_host = Some(preset.default_api_host.to_string()).filter(|h| !h.is_empty());
    }

    pub fn host(&self) -> String {
        self.api_host
            .clone()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| self.preset().default_api_host.to_string())
    }

    /// `isWebSearchReady`: enabled, has what the provider needs and a well-formed host.
    pub fn is_ready(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let preset = self.preset();
        if preset.requires_api_key
            && self
                .api_key
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return false;
        }
        if preset.id == "searxng"
            && self
                .api_host
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return false;
        }
        match self.api_host.as_deref().filter(|h| !h.is_empty()) {
            Some(host) => host.starts_with("http://") || host.starts_with("https://"),
            None => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub content: String,
}

/// `(method, url, headers, body)` of the request a provider needs.
pub struct SearchRequest {
    pub post: bool,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

pub fn build_request(
    config: &WebSearchConfig,
    query: &str,
    max_results: u32,
) -> Result<SearchRequest, String> {
    let host = config.host();
    let host = host.trim_end_matches('/');
    let key = config.api_key.clone().unwrap_or_default();
    let json = ("Content-Type".to_string(), "application/json".to_string());
    let bearer = ("Authorization".to_string(), format!("Bearer {key}"));
    Ok(match config.provider_id.as_str() {
        "tavily" => SearchRequest {
            post: true,
            url: format!("{host}/search"),
            headers: vec![json, bearer],
            body: Some(
                serde_json::json!({"query": query, "max_results": max_results, "search_depth": "basic"}),
            ),
        },
        "exa" => SearchRequest {
            post: true,
            url: format!("{host}/search"),
            headers: vec![json, ("x-api-key".into(), key)],
            body: Some(
                serde_json::json!({"query": query, "numResults": max_results, "contents": {"text": true}}),
            ),
        },
        "bocha" => SearchRequest {
            post: true,
            url: format!("{host}/v1/web-search"),
            headers: vec![json, bearer],
            body: Some(serde_json::json!({"query": query, "count": max_results, "summary": true})),
        },
        "zhipu" => SearchRequest {
            post: true,
            url: format!("{host}/web_search"),
            headers: vec![json, bearer],
            body: Some(serde_json::json!({"search_query": query, "search_engine": "search_std"})),
        },
        "searxng" => {
            if host.is_empty() {
                return Err("SearXNG requires an API Host to be configured".into());
            }
            SearchRequest {
                post: false,
                url: format!("{host}/search?q={}&format=json", urlencode(query)),
                headers: Vec::new(),
                body: None,
            }
        }
        other => return Err(format!("Unsupported web search provider: {other}")),
    })
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Normalises a provider response to `{title, url, content}` rows.
pub fn parse_response(
    provider_id: &str,
    payload: &serde_json::Value,
    max_results: u32,
) -> Vec<SearchResult> {
    let text = |v: &serde_json::Value, key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    let rows =
        |v: Option<&serde_json::Value>| v.and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let mut out: Vec<SearchResult> = match provider_id {
        "tavily" | "searxng" => rows(payload.get("results"))
            .iter()
            .map(|r| SearchResult {
                title: text(r, "title"),
                url: text(r, "url"),
                content: text(r, "content"),
            })
            .collect(),
        "exa" => rows(payload.get("results"))
            .iter()
            .map(|r| SearchResult {
                title: text(r, "title"),
                url: text(r, "url"),
                content: text(r, "text"),
            })
            .collect(),
        "bocha" => rows(payload.get("webPages").and_then(|w| w.get("value")))
            .iter()
            .map(|r| SearchResult {
                title: text(r, "name"),
                url: text(r, "url"),
                content: Some(text(r, "summary"))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| text(r, "snippet")),
            })
            .collect(),
        "zhipu" => rows(payload.get("search_result"))
            .iter()
            .map(|r| SearchResult {
                title: text(r, "title"),
                url: text(r, "link"),
                content: text(r, "content"),
            })
            .collect(),
        _ => Vec::new(),
    };
    // providers without a count parameter return more than asked for
    out.truncate(max_results as usize);
    out
}

pub async fn web_search(
    config: &WebSearchConfig,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    if !config.is_ready() {
        return Err("网页搜索尚未配置完成".into());
    }
    let max = config.max_results.unwrap_or(DEFAULT_MAX_RESULTS);
    let request = build_request(config, query, max)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut builder = if request.post {
        client.post(&request.url)
    } else {
        client.get(&request.url)
    };
    for (k, v) in &request.headers {
        builder = builder.header(k, v);
    }
    if let Some(body) = &request.body {
        builder = builder.json(body);
    }
    let resp = builder.send().await.map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "HTTP {status}: {}",
            text.chars().take(200).collect::<String>()
        ));
    }
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))?;
    Ok(parse_response(&config.provider_id, &payload, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(id: &str) -> WebSearchConfig {
        WebSearchConfig {
            provider_id: id.into(),
            api_key: Some("k".into()),
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn readiness_requires_what_the_provider_needs() {
        assert!(cfg("tavily").is_ready());
        let mut c = cfg("tavily");
        c.api_key = None;
        assert!(!c.is_ready());
        c = cfg("tavily");
        c.enabled = false;
        assert!(!c.is_ready());
        c = cfg("tavily");
        c.api_host = Some("not a url".into());
        assert!(!c.is_ready());
        let mut s = WebSearchConfig {
            provider_id: "searxng".into(),
            enabled: true,
            ..Default::default()
        };
        assert!(!s.is_ready());
        s.api_host = Some("https://searx.example".into());
        assert!(s.is_ready());
    }

    #[test]
    fn switching_provider_resets_key_and_host() {
        let mut c = cfg("tavily");
        c.api_host = Some("https://x".into());
        c.switch_provider("exa");
        assert_eq!(
            (
                c.provider_id.as_str(),
                c.api_key.clone(),
                c.api_host.as_deref()
            ),
            ("exa", None, Some("https://api.exa.ai"))
        );
        c.switch_provider("searxng");
        assert_eq!(c.api_host, None);
        assert!(!search_preset("searxng").requires_api_key);
    }

    #[test]
    fn normalize_clamps_and_repairs() {
        let mut c = WebSearchConfig {
            provider_id: "bing".into(),
            max_results: Some(99),
            api_host: Some("  ".into()),
            ..Default::default()
        };
        c.normalize();
        assert_eq!(
            (c.provider_id.as_str(), c.max_results, c.api_host.clone()),
            ("tavily", Some(20), None)
        );
        c.max_results = Some(0);
        c.normalize();
        assert_eq!(c.max_results, Some(1));
    }

    #[test]
    fn requests_follow_each_provider_api() {
        let r = build_request(&cfg("tavily"), "rust gpui", 7).unwrap();
        assert_eq!(r.url, "https://api.tavily.com/search");
        assert!(
            r.post
                && r.headers
                    .iter()
                    .any(|(k, v)| k == "Authorization" && v == "Bearer k")
        );
        assert_eq!(r.body.unwrap()["max_results"], 7);
        let r = build_request(&cfg("exa"), "q", 3).unwrap();
        assert!(r.headers.iter().any(|(k, v)| k == "x-api-key" && v == "k"));
        assert_eq!(r.body.unwrap()["numResults"], 3);
        let r = build_request(&cfg("bocha"), "q", 3).unwrap();
        assert_eq!(r.url, "https://api.bochaai.com/v1/web-search");
        let r = build_request(&cfg("zhipu"), "q", 3).unwrap();
        assert_eq!(r.url, "https://open.bigmodel.cn/api/paas/v4/web_search");
        assert_eq!(r.body.unwrap()["search_engine"], "search_std");
        let mut s = cfg("searxng");
        s.api_host = Some("https://searx.example/".into());
        let r = build_request(&s, "a b&c", 3).unwrap();
        assert_eq!(
            r.url,
            "https://searx.example/search?q=a%20b%26c&format=json"
        );
        assert!(!r.post);
        assert!(build_request(&cfg("searxng"), "q", 3).is_err());
        assert!(build_request(&cfg("nope"), "q", 3).is_err());
    }

    #[test]
    fn responses_are_normalised() {
        let v = serde_json::json!({"results": [{"title": "t", "url": "u", "content": "c"}, {"title": "t2", "url": "u2", "content": "c2"}]});
        let rows = parse_response("tavily", &v, 1);
        assert_eq!(
            rows,
            vec![SearchResult {
                title: "t".into(),
                url: "u".into(),
                content: "c".into()
            }]
        );
        let exa = serde_json::json!({"results": [{"title": "t", "url": "u", "text": "body"}]});
        assert_eq!(parse_response("exa", &exa, 5)[0].content, "body");
        let bocha =
            serde_json::json!({"webPages": {"value": [{"name": "n", "url": "u", "snippet": "s"}]}});
        assert_eq!(parse_response("bocha", &bocha, 5)[0].content, "s");
        let zhipu =
            serde_json::json!({"search_result": [{"title": "t", "link": "l", "content": "c"}]});
        assert_eq!(parse_response("zhipu", &zhipu, 5)[0].url, "l");
        assert!(parse_response("tavily", &serde_json::json!({}), 5).is_empty());
    }
}
