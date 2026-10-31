//! Model providers (`ProviderConfig`, `PROVIDER_PRESETS`, the connection probe and model discovery of Netcatty).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AdvancedParams {
    pub max_tokens: Option<u32>,
    /// 0–2
    pub temperature: Option<f64>,
    /// 0–1
    pub top_p: Option<f64>,
    /// -2–2
    pub frequency_penalty: Option<f64>,
    /// -2–2
    pub presence_penalty: Option<f64>,
    /// `low` | `medium` | `high`; unset = the API default (not sent).
    pub reasoning_effort: Option<String>,
}

impl AdvancedParams {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Clamps every value into the range the form accepts.
    pub fn clamped(mut self) -> Self {
        self.temperature = self.temperature.map(|v| v.clamp(0., 2.));
        self.top_p = self.top_p.map(|v| v.clamp(0., 1.));
        self.frequency_penalty = self.frequency_penalty.map(|v| v.clamp(-2., 2.));
        self.presence_penalty = self.presence_penalty.map(|v| v.clamp(-2., 2.));
        self.max_tokens = self.max_tokens.filter(|v| *v > 0);
        self.reasoning_effort = self
            .reasoning_effort
            .filter(|v| matches!(v.as_str(), "low" | "medium" | "high"));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    /// Overrides the wire protocol (`openai` | `anthropic` | `google`); unset follows `provider_id`.
    pub style: Option<String>,
    /// `chat` (default) or `responses`; only used by the OpenAI style.
    pub openai_api: Option<String>,
    pub icon_id: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    /// Header values are kept in the keychain; this map is only filled in memory.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_headers: BTreeMap<String, String>,
    pub has_secret_headers: bool,
    pub enabled: bool,
    pub skip_tls_verify: bool,
    /// User override of the context window, in tokens.
    pub context_window: Option<u64>,
    pub model_context_windows: BTreeMap<String, u64>,
    pub advanced_params: Option<AdvancedParams>,
}

pub struct ProviderPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub has_models_endpoint: bool,
    pub default_models: &'static [&'static str],
}

pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        has_models_endpoint: true,
        default_models: &[],
    },
    ProviderPreset {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com",
        has_models_endpoint: true,
        default_models: &[],
    },
    ProviderPreset {
        id: "google",
        name: "Google AI",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        has_models_endpoint: false,
        default_models: &[],
    },
    ProviderPreset {
        id: "ollama",
        name: "Ollama",
        base_url: "http://localhost:11434/v1",
        has_models_endpoint: true,
        default_models: &[],
    },
    ProviderPreset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        has_models_endpoint: true,
        default_models: &[],
    },
    ProviderPreset {
        id: "qwen",
        name: "Qwen",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        has_models_endpoint: true,
        default_models: &[
            "qwen3.7-plus",
            "qwen3.7-max",
            "qwen3.6-plus",
            "qwen3.6-flash",
            "qwen3.6-max-preview",
            "qwen3.5-plus",
            "qwen3-coder-plus",
            "qwen3-coder-flash",
            "qwen-plus",
            "qwen-plus-latest",
        ],
    },
    ProviderPreset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        has_models_endpoint: true,
        default_models: &[
            "deepseek-v4-flash",
            "deepseek-v4-pro",
            "deepseek-chat",
            "deepseek-reasoner",
        ],
    },
    ProviderPreset {
        id: "kimi",
        name: "Kimi",
        base_url: "https://api.moonshot.ai/v1",
        has_models_endpoint: true,
        default_models: &[
            "kimi-k2.6",
            "kimi-k2.5",
            "moonshot-v1-128k",
            "moonshot-v1-32k",
            "moonshot-v1-8k",
        ],
    },
    ProviderPreset {
        id: "zhipu",
        name: "Zhipu",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        has_models_endpoint: true,
        default_models: &[
            "glm-5.1",
            "glm-5",
            "glm-5-turbo",
            "glm-4.7",
            "glm-4.7-flash",
            "glm-4.6",
            "glm-4.5",
            "glm-4.5-air",
        ],
    },
    ProviderPreset {
        id: "doubao",
        name: "Doubao",
        base_url: "https://ark.cn-beijing.volces.com/api/v3",
        has_models_endpoint: true,
        default_models: &[
            "doubao-seed-2-0-pro-260215",
            "doubao-seed-2-0-lite-260215",
            "doubao-seed-2-0-mini-260215",
            "doubao-seed-2-0-code-preview-260215",
        ],
    },
    ProviderPreset {
        id: "mimo",
        name: "Xiaomi MiMo",
        base_url: "https://api.xiaomimimo.com/v1",
        has_models_endpoint: true,
        default_models: &["mimo-v2.5-pro", "mimo-v2.5"],
    },
    ProviderPreset {
        id: "custom",
        name: "Custom",
        base_url: "",
        has_models_endpoint: false,
        default_models: &[],
    },
];

pub fn preset(provider_id: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|p| p.id == provider_id)
}

pub const STYLES: [&str; 3] = ["openai", "anthropic", "google"];

/// `resolveProviderStyle`: the explicit style, else the one the provider type implies.
pub fn resolve_style(p: &ProviderConfig) -> &'static str {
    let explicit = p
        .style
        .as_deref()
        .and_then(|s| STYLES.iter().copied().find(|k| *k == s));
    explicit.unwrap_or(match p.provider_id.as_str() {
        "anthropic" => "anthropic",
        "google" => "google",
        _ => "openai",
    })
}

/// `resolveOpenAIApi`: anything but an explicit `responses` stays on Chat Completions.
pub fn resolve_openai_api(p: &ProviderConfig) -> &'static str {
    if p.openai_api.as_deref() == Some("responses") {
        "responses"
    } else {
        "chat"
    }
}

/// A fresh, disabled provider of the given type with the preset's endpoint and a random id.
pub fn new_provider(provider_id: &str) -> ProviderConfig {
    let preset = preset(provider_id).unwrap_or(&PROVIDER_PRESETS[PROVIDER_PRESETS.len() - 1]);
    ProviderConfig {
        id: format!(
            "provider_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        ),
        provider_id: preset.id.to_string(),
        name: preset.name.to_string(),
        base_url: Some(preset.base_url.to_string()).filter(|u| !u.is_empty()),
        default_model: preset.default_models.first().map(|m| m.to_string()),
        ..Default::default()
    }
}

/// The base URL a request goes to: the configured one, else the preset's.
pub fn effective_base_url(p: &ProviderConfig) -> String {
    let configured = p
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let url = configured
        .map(str::to_string)
        .or_else(|| preset(&p.provider_id).map(|x| x.base_url.to_string()))
        .unwrap_or_default();
    if p.provider_id == "ollama" {
        normalize_ollama_base_url(&url)
    } else {
        url
    }
}

/// Ollama Cloud's OpenAI-compatible host is `https://ollama.com/v1`; a bare origin or `/api` hits the website.
pub fn normalize_ollama_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower == "https://ollama.com" || lower == "http://ollama.com" {
        return format!("{trimmed}/v1");
    }
    if lower == "https://ollama.com/api" || lower == "http://ollama.com/api" {
        return format!("{}/v1", &trimmed[..trimmed.len() - 4]);
    }
    if trimmed.is_empty() {
        base_url.to_string()
    } else {
        trimmed.to_string()
    }
}

/// What the single-provider chat client (`ai_config`) should talk to for this provider. Anthropic and Google are
/// reached through their OpenAI-compatible endpoints there; the exact style is kept in the provider entry.
pub fn legacy_endpoint(p: &ProviderConfig) -> (&'static str, String) {
    let base = effective_base_url(p);
    match resolve_style(p) {
        "anthropic" => (
            "custom",
            format!("{}/v1", base.trim_end_matches('/').trim_end_matches("/v1")),
        ),
        "google" => (
            "custom",
            if base.ends_with("/openai") {
                base
            } else {
                format!("{}/openai", base.trim_end_matches('/'))
            },
        ),
        _ if p.provider_id == "ollama" => ("ollama", base),
        _ if p.provider_id == "openai" => ("openai", base),
        _ => ("custom", base),
    }
}

/// Models offered in the "default model" picker before (or without) discovery.
pub fn preset_models(p: &ProviderConfig) -> Vec<String> {
    preset(&p.provider_id)
        .map(|x| x.default_models.iter().map(|m| m.to_string()).collect())
        .unwrap_or_default()
}

/// Clamps `contextWindow` like `sanitizeContextWindow`: positive whole tokens or nothing.
pub fn sanitize_context_window(value: &str) -> Result<Option<u64>, ()> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(None);
    }
    match v.parse::<u64>() {
        Ok(n) if n >= 1024 => Ok(Some(n)),
        _ => Err(()),
    }
}

// ---------------------------------------------------------------------------
// Discovery / connection probe
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeHealth {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub health: ProbeHealth,
    pub latency_ms: u128,
    pub status: u16,
    pub model_count: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeInputIssue {
    MissingBaseUrl,
    MissingApiKey,
}

pub fn validate_probe_inputs(p: &ProviderConfig) -> Result<(), ProbeInputIssue> {
    if effective_base_url(p).trim().is_empty() {
        return Err(ProbeInputIssue::MissingBaseUrl);
    }
    if p.provider_id != "ollama" && p.api_key.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(ProbeInputIssue::MissingApiKey);
    }
    Ok(())
}

/// `buildProviderProbeUrl`: `/v1/models` on a base that already ends in `/v1` is not doubled.
pub fn models_url(base_url: &str, style: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let mut path = if style == "anthropic" {
        "/v1/models"
    } else {
        "/models"
    }
    .to_string();
    if base.to_ascii_lowercase().ends_with("/v1") && path.starts_with("/v1") {
        path = path[3..].to_string();
    }
    format!("{base}{path}")
}

fn http_client(p: &ProviderConfig, timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .danger_accept_invalid_certs(p.skip_tls_verify)
        .build()
        .map_err(|e| format!("初始化请求失败: {e}"))
}

fn discovery_request(p: &ProviderConfig, client: &reqwest::Client) -> reqwest::RequestBuilder {
    let style = resolve_style(p);
    let mut req = client
        .get(models_url(&effective_base_url(p), style))
        .header("Content-Type", "application/json");
    let key = p.api_key.as_deref().map(str::trim).unwrap_or("");
    if !key.is_empty() {
        req = match style {
            "anthropic" => req
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
            "google" => req.header("x-goog-api-key", key),
            _ => req.header("Authorization", format!("Bearer {key}")),
        };
    }
    for (name, value) in &p.custom_headers {
        if !name.trim().is_empty() {
            req = req.header(name.trim(), value);
        }
    }
    req
}

/// Model ids of a `/models` payload: OpenAI-style `data[].id` or Google's `models[].name` (`models/` stripped).
pub fn parse_model_ids(payload: &serde_json::Value) -> Vec<String> {
    let list = payload
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| payload.get("models").and_then(|v| v.as_array()));
    let mut ids: Vec<String> = list
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(|v| v.as_str())
                        .or_else(|| m.get("name").and_then(|v| v.as_str()))
                })
                .map(|id| id.strip_prefix("models/").unwrap_or(id).to_string())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    ids
}

/// Context windows advertised in a model list (`context_length`, `context_window`, `inputTokenLimit`, ...).
pub fn parse_context_windows(payload: &serde_json::Value) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let list = payload
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| payload.get("models").and_then(|v| v.as_array()));
    for m in list.into_iter().flatten() {
        let Some(id) = m
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| m.get("name").and_then(|v| v.as_str()))
        else {
            continue;
        };
        let window = [
            "context_length",
            "context_window",
            "max_context_length",
            "inputTokenLimit",
            "max_input_tokens",
        ]
        .iter()
        .find_map(|k| m.get(*k).and_then(|v| v.as_u64()));
        if let Some(w) = window.filter(|w| *w > 0) {
            out.insert(id.strip_prefix("models/").unwrap_or(id).to_string(), w);
        }
    }
    out
}

pub struct DiscoveredModels {
    pub ids: Vec<String>,
    pub context_windows: BTreeMap<String, u64>,
}

/// `GET /models` for the provider.
pub async fn discover_models(p: &ProviderConfig) -> Result<DiscoveredModels, String> {
    validate_probe_inputs(p).map_err(|issue| match issue {
        ProbeInputIssue::MissingBaseUrl => "请先填写 API 地址".to_string(),
        ProbeInputIssue::MissingApiKey => "请先填写 API Key".to_string(),
    })?;
    let client = http_client(p, Duration::from_secs(15))?;
    let resp = discovery_request(p, &client)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "获取模型列表失败 (HTTP {status}): {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    let payload: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))?;
    Ok(DiscoveredModels {
        ids: parse_model_ids(&payload),
        context_windows: parse_context_windows(&payload),
    })
}

/// The settings "Test" button: a `/models` round trip classified green / yellow / red.
pub async fn probe(p: &ProviderConfig) -> ProbeResult {
    let started = Instant::now();
    let failed = |status: u16, error: String| ProbeResult {
        health: ProbeHealth::Error,
        latency_ms: started.elapsed().as_millis(),
        status,
        model_count: None,
        error: Some(error),
    };
    let client = match http_client(p, Duration::from_secs(15)) {
        Ok(c) => c,
        Err(e) => return failed(0, e),
    };
    let resp = match discovery_request(p, &client).send().await {
        Ok(r) => r,
        Err(e) => return failed(0, e.to_string()),
    };
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    classify_probe(status.as_u16(), started.elapsed().as_millis(), &body)
}

/// `classifyProviderProbeResponse`: yellow covers slow-but-successful replies and empty catalogs.
pub fn classify_probe(status: u16, latency_ms: u128, body: &str) -> ProbeResult {
    if !(200..300).contains(&status) {
        return ProbeResult {
            health: ProbeHealth::Error,
            latency_ms,
            status,
            model_count: None,
            error: Some(format!("HTTP {status}")),
        };
    }
    let count = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .map(|v| parse_model_ids(&v).len());
    match count {
        None => ProbeResult {
            health: ProbeHealth::Warn,
            latency_ms,
            status,
            model_count: None,
            error: Some("Unexpected response body".into()),
        },
        Some(n) if n == 0 || latency_ms >= 3000 => ProbeResult {
            health: ProbeHealth::Warn,
            latency_ms,
            status,
            model_count: Some(n),
            error: None,
        },
        Some(n) => ProbeResult {
            health: ProbeHealth::Ok,
            latency_ms,
            status,
            model_count: Some(n),
            error: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_follows_provider_type_unless_overridden() {
        let mut p = new_provider("anthropic");
        assert_eq!(resolve_style(&p), "anthropic");
        p.style = Some("openai".into());
        assert_eq!(resolve_style(&p), "openai");
        p.style = Some("bogus".into());
        assert_eq!(resolve_style(&p), "anthropic");
        assert_eq!(resolve_style(&new_provider("deepseek")), "openai");
        assert_eq!(resolve_style(&new_provider("google")), "google");
    }

    #[test]
    fn openai_api_defaults_to_chat_completions() {
        let mut p = new_provider("openai");
        assert_eq!(resolve_openai_api(&p), "chat");
        p.openai_api = Some("responses".into());
        assert_eq!(resolve_openai_api(&p), "responses");
        p.openai_api = Some("weird".into());
        assert_eq!(resolve_openai_api(&p), "chat");
    }

    #[test]
    fn new_provider_uses_the_preset() {
        let p = new_provider("qwen");
        assert_eq!(p.name, "Qwen");
        assert_eq!(
            p.base_url.as_deref(),
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1")
        );
        assert_eq!(p.default_model.as_deref(), Some("qwen3.7-plus"));
        assert!(!p.enabled);
        assert!(p.id.starts_with("provider_"));
        assert_eq!(new_provider("custom").base_url, None);
        assert_eq!(new_provider("nope").provider_id, "custom");
    }

    #[test]
    fn ollama_cloud_urls_get_the_v1_prefix() {
        assert_eq!(
            normalize_ollama_base_url("https://ollama.com"),
            "https://ollama.com/v1"
        );
        assert_eq!(
            normalize_ollama_base_url("https://ollama.com/api/"),
            "https://ollama.com/v1"
        );
        assert_eq!(
            normalize_ollama_base_url("http://localhost:11434/v1/"),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn models_url_does_not_double_the_version_segment() {
        assert_eq!(
            models_url("https://api.openai.com/v1/", "openai"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_url("https://api.anthropic.com", "anthropic"),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            models_url("https://proxy.test/v1", "anthropic"),
            "https://proxy.test/v1/models"
        );
        assert_eq!(
            models_url("https://generativelanguage.googleapis.com/v1beta", "google"),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn legacy_endpoint_maps_styles_onto_openai_compatible_urls() {
        assert_eq!(
            legacy_endpoint(&new_provider("openai")),
            ("openai", "https://api.openai.com/v1".into())
        );
        assert_eq!(legacy_endpoint(&new_provider("ollama")).0, "ollama");
        assert_eq!(
            legacy_endpoint(&new_provider("anthropic")),
            ("custom", "https://api.anthropic.com/v1".into())
        );
        assert_eq!(
            legacy_endpoint(&new_provider("google")),
            (
                "custom",
                "https://generativelanguage.googleapis.com/v1beta/openai".into()
            )
        );
        assert_eq!(
            legacy_endpoint(&new_provider("deepseek")),
            ("custom", "https://api.deepseek.com/v1".into())
        );
    }

    #[test]
    fn probe_inputs_need_a_url_and_a_key_except_for_ollama() {
        let mut p = new_provider("openai");
        assert_eq!(
            validate_probe_inputs(&p),
            Err(ProbeInputIssue::MissingApiKey)
        );
        p.api_key = Some("sk-x".into());
        assert_eq!(validate_probe_inputs(&p), Ok(()));
        assert_eq!(validate_probe_inputs(&new_provider("ollama")), Ok(()));
        assert_eq!(
            validate_probe_inputs(&new_provider("custom")),
            Err(ProbeInputIssue::MissingBaseUrl)
        );
    }

    #[test]
    fn model_payloads_parse_for_openai_and_google_shapes() {
        let openai =
            serde_json::json!({"data": [{"id": "b", "context_length": 8192}, {"id": "a"}]});
        assert_eq!(parse_model_ids(&openai), vec!["a", "b"]);
        assert_eq!(parse_context_windows(&openai).get("b"), Some(&8192));
        let google = serde_json::json!({"models": [{"name": "models/gemini-2.5-pro", "inputTokenLimit": 1048576}]});
        assert_eq!(parse_model_ids(&google), vec!["gemini-2.5-pro"]);
        assert_eq!(
            parse_context_windows(&google).get("gemini-2.5-pro"),
            Some(&1048576)
        );
    }

    #[test]
    fn probe_classification() {
        assert_eq!(
            classify_probe(200, 120, r#"{"data":[{"id":"m"}]}"#).health,
            ProbeHealth::Ok
        );
        assert_eq!(
            classify_probe(200, 4000, r#"{"data":[{"id":"m"}]}"#).health,
            ProbeHealth::Warn
        );
        assert_eq!(
            classify_probe(200, 10, r#"{"data":[]}"#).health,
            ProbeHealth::Warn
        );
        assert_eq!(classify_probe(200, 10, "<html>").health, ProbeHealth::Warn);
        let err = classify_probe(401, 10, "");
        assert_eq!(err.health, ProbeHealth::Error);
        assert_eq!(err.error.as_deref(), Some("HTTP 401"));
    }

    #[test]
    fn advanced_params_are_clamped() {
        let p = AdvancedParams {
            temperature: Some(5.),
            top_p: Some(-1.),
            frequency_penalty: Some(9.),
            presence_penalty: Some(-9.),
            max_tokens: Some(0),
            reasoning_effort: Some("extreme".into()),
        }
        .clamped();
        assert_eq!(
            (
                p.temperature,
                p.top_p,
                p.frequency_penalty,
                p.presence_penalty
            ),
            (Some(2.), Some(0.), Some(2.), Some(-2.))
        );
        assert_eq!((p.max_tokens, p.reasoning_effort), (None, None));
        assert!(AdvancedParams::default().is_empty());
    }

    #[test]
    fn context_window_input() {
        assert_eq!(sanitize_context_window(" "), Ok(None));
        assert_eq!(sanitize_context_window("128000"), Ok(Some(128000)));
        assert_eq!(sanitize_context_window("12"), Err(()));
        assert_eq!(sanitize_context_window("abc"), Err(()));
    }
}
