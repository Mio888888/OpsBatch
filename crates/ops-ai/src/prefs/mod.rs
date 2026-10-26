//! AI preferences ported from Netcatty's AI settings (<https://github.com/binaricat/Netcatty>): multiple model
//! providers, external agent CLIs, tool access (MCP / skills, privacy, external MCP, quick messages), web search and
//! safety (permission mode, command blocklist, timeouts, permission grants).
//!
//! Everything lives in one JSON document in the `settings` table (key `ai_prefs`, camelCase like Netcatty's own
//! storage) so the Tauri build, which ignores unknown keys, keeps reading its `ai_config` row. Secrets (provider and
//! search API keys, header values, Cursor key) are moved to the keychain facade; the JSON only holds a placeholder.
//! The enabled provider is mirrored into the legacy `ai_config` row, which is what the chat panel and the batch
//! window talk to.

pub mod agents;
pub mod external_mcp;
pub mod privacy;
pub mod provider;
pub mod quick;
pub mod safety;
pub mod search;
pub mod user_skills;

use std::collections::BTreeMap;

use ops_core::db::Database;
use ops_core::security::SECRET_PLACEHOLDER;
use rusqlite::params;
use serde::{Deserialize, Serialize};

pub use agents::{CodebuddyOptions, ExternalAgentConfig};
pub use external_mcp::{ExternalMcpHttpSettings, ExternalMcpSettings};
pub use provider::{AdvancedParams, ProviderConfig};
pub use quick::QuickMessage;
pub use safety::PermissionGrantRule;
pub use search::WebSearchConfig;

const PREFS_KEY: &str = "ai_prefs";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    Observer,
    #[default]
    Confirm,
    Auto,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 3] = [PermissionMode::Observer, PermissionMode::Confirm, PermissionMode::Auto];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observer => "observer",
            Self::Confirm => "confirm",
            Self::Auto => "auto",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolIntegrationMode {
    #[default]
    Mcp,
    Skills,
}

impl ToolIntegrationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skills => "skills",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "skills" {
            Self::Skills
        } else {
            Self::Mcp
        }
    }
}

/// "Sensitive data" redaction applied to what is sent to a model (`AIPrivacySection`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PrivacySettings {
    pub enabled: bool,
    pub redact_private_ips: bool,
    pub cidrs: Vec<String>,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AiPrefs {
    pub providers: Vec<ProviderConfig>,
    pub active_provider_id: String,
    pub active_model_id: String,
    pub global_permission_mode: PermissionMode,
    pub tool_integration_mode: ToolIntegrationMode,
    pub external_agents: Vec<ExternalAgentConfig>,
    pub default_agent_id: String,
    pub command_blocklist: Vec<String>,
    pub command_timeout: u32,
    pub response_idle_timeout: u32,
    pub max_iterations: u32,
    pub web_search: Option<WebSearchConfig>,
    pub quick_messages: Vec<QuickMessage>,
    pub show_terminal_selection_ai_action: bool,
    pub permission_grants: Vec<PermissionGrantRule>,
    pub privacy: PrivacySettings,
    pub external_mcp: ExternalMcpSettings,
    /// Per-agent last used model / provider (`STORAGE_KEY_AI_AGENT_MODEL_MAP` / `..._PROVIDER_MAP`).
    pub agent_model_map: BTreeMap<String, String>,
    pub agent_provider_map: BTreeMap<String, String>,
}

impl Default for AiPrefs {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            active_provider_id: String::new(),
            active_model_id: String::new(),
            global_permission_mode: PermissionMode::Confirm,
            tool_integration_mode: ToolIntegrationMode::Mcp,
            external_agents: Vec::new(),
            default_agent_id: "catty".into(),
            command_blocklist: safety::default_command_blocklist(),
            command_timeout: safety::DEFAULT_COMMAND_TIMEOUT_SECONDS,
            response_idle_timeout: safety::DEFAULT_RESPONSE_IDLE_TIMEOUT_SECONDS,
            max_iterations: safety::DEFAULT_MAX_ITERATIONS,
            web_search: None,
            quick_messages: Vec::new(),
            show_terminal_selection_ai_action: true,
            permission_grants: Vec::new(),
            privacy: PrivacySettings::default(),
            external_mcp: ExternalMcpSettings::default(),
            agent_model_map: BTreeMap::new(),
            agent_provider_map: BTreeMap::new(),
        }
    }
}

impl AiPrefs {
    pub fn active_provider(&self) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == self.active_provider_id)
    }

    pub fn provider_mut(&mut self, id: &str) -> Option<&mut ProviderConfig> {
        self.providers.iter_mut().find(|p| p.id == id)
    }

    pub fn agent_mut(&mut self, id: &str) -> Option<&mut ExternalAgentConfig> {
        self.external_agents.iter_mut().find(|a| a.id == id)
    }

    /// `removeProvider` of `useAISettingsState`: drops the provider and every reference to it.
    pub fn remove_provider(&mut self, id: &str) {
        self.providers.retain(|p| p.id != id);
        if self.active_provider_id == id {
            self.active_provider_id.clear();
            self.active_model_id.clear();
        }
        let orphaned: Vec<String> = self.agent_provider_map.iter().filter(|(_, provider)| provider.as_str() == id).map(|(agent, _)| agent.clone()).collect();
        for agent in orphaned {
            self.agent_provider_map.remove(&agent);
            self.agent_model_map.remove(&agent);
        }
    }

    /// The provider switch of `SettingsAITab`: at most one provider is enabled, and it is the active one.
    pub fn set_provider_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            let default_model = self.providers.iter().find(|p| p.id == id).and_then(|p| p.default_model.clone());
            self.active_provider_id = id.to_string();
            if let Some(model) = default_model.filter(|m| !m.is_empty()) {
                self.active_model_id = model;
            }
            for p in &mut self.providers {
                p.enabled = p.id == id;
            }
        } else {
            if self.active_provider_id == id {
                self.active_provider_id.clear();
                self.active_model_id.clear();
            }
            if let Some(p) = self.provider_mut(id) {
                p.enabled = false;
            }
        }
    }

    /// Normalizes values that came from disk or the form (clamped timeouts, deduped quick messages, ...).
    pub fn normalize(&mut self) {
        self.command_timeout = safety::normalize_command_timeout(self.command_timeout as i64);
        self.response_idle_timeout = safety::normalize_response_idle_timeout(self.response_idle_timeout as i64);
        self.max_iterations = safety::normalize_max_iterations(self.max_iterations as i64);
        self.quick_messages = quick::sanitize_quick_messages(std::mem::take(&mut self.quick_messages));
        self.external_mcp.normalize();
        if let Some(search) = &mut self.web_search {
            search.normalize();
        }
        if self.default_agent_id.trim().is_empty() {
            self.default_agent_id = "catty".into();
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

fn provider_secret_name(id: &str) -> String {
    format!("ai-provider:{id}")
}
fn provider_headers_secret_name(id: &str) -> String {
    format!("ai-provider-headers:{id}")
}
const WEB_SEARCH_SECRET: &str = "ai-web-search";
const MCP_HTTP_TOKEN_SECRET: &str = "ai-external-mcp-http-token";
fn agent_secret_name(id: &str) -> String {
    format!("ai-agent:{id}")
}

fn read_secret(name: &str) -> String {
    ops_core::keychain::get_api_key(name).unwrap_or_default()
}

/// Moves a secret into the keychain and returns what the JSON should hold in its place.
fn stash_secret(name: &str, value: &Option<String>) -> Result<Option<String>, String> {
    match value.as_deref().map(str::trim) {
        None | Some("") => {
            let _ = ops_core::keychain::delete_api_key(name);
            Ok(None)
        }
        Some(v) if v == SECRET_PLACEHOLDER => Ok(Some(SECRET_PLACEHOLDER.to_string())),
        Some(v) => {
            ops_core::keychain::store_api_key(name, v)?;
            Ok(Some(SECRET_PLACEHOLDER.to_string()))
        }
    }
}

/// Fills the placeholders of a decoded document with the real secrets.
fn reveal_secrets(prefs: &mut AiPrefs) {
    for p in &mut prefs.providers {
        if p.api_key.as_deref() == Some(SECRET_PLACEHOLDER) {
            let key = read_secret(&provider_secret_name(&p.id));
            p.api_key = Some(key).filter(|k| !k.is_empty());
        }
        if p.has_secret_headers {
            if let Ok(json) = ops_core::keychain::get_api_key(&provider_headers_secret_name(&p.id)) {
                p.custom_headers = serde_json::from_str(&json).unwrap_or_default();
            }
        }
    }
    if let Some(search) = &mut prefs.web_search {
        if search.api_key.as_deref() == Some(SECRET_PLACEHOLDER) {
            search.api_key = Some(read_secret(WEB_SEARCH_SECRET)).filter(|k| !k.is_empty());
        }
    }
    for a in &mut prefs.external_agents {
        if a.api_key.as_deref() == Some(SECRET_PLACEHOLDER) {
            a.api_key = Some(read_secret(&agent_secret_name(&a.id))).filter(|k| !k.is_empty());
        }
    }
    prefs.external_mcp.http.token = read_secret(MCP_HTTP_TOKEN_SECRET);
}

/// A copy of `prefs` that is safe to write to SQLite; the secrets go to the keychain on the way.
fn seal_secrets(prefs: &AiPrefs) -> Result<AiPrefs, String> {
    let mut sealed = prefs.clone();
    for p in &mut sealed.providers {
        p.api_key = stash_secret(&provider_secret_name(&p.id), &p.api_key)?;
        let headers = std::mem::take(&mut p.custom_headers);
        if headers.is_empty() {
            let _ = ops_core::keychain::delete_api_key(&provider_headers_secret_name(&p.id));
            p.has_secret_headers = false;
        } else {
            let json = serde_json::to_string(&headers).map_err(|e| e.to_string())?;
            ops_core::keychain::store_api_key(&provider_headers_secret_name(&p.id), &json)?;
            p.has_secret_headers = true;
        }
    }
    if let Some(search) = &mut sealed.web_search {
        search.api_key = stash_secret(WEB_SEARCH_SECRET, &search.api_key)?;
    }
    for a in &mut sealed.external_agents {
        a.api_key = stash_secret(&agent_secret_name(&a.id), &a.api_key)?;
    }
    stash_secret(MCP_HTTP_TOKEN_SECRET, &Some(sealed.external_mcp.http.token.clone()))?;
    Ok(sealed)
}

fn read_row(db: &Database, key: &str) -> Result<Option<String>, String> {
    let conn = db.pool.get().map_err(|e| e.to_string())?;
    match conn.query_row("SELECT value FROM settings WHERE key=?1", params![key], |r| r.get::<_, String>(0)) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Loads the preferences, secrets included. A database that only has the single-provider `ai_config` row of the
/// Tauri build gets that provider listed (and active) so nothing configured there disappears.
pub fn load_prefs(db: &Database) -> Result<AiPrefs, String> {
    let mut prefs = match read_row(db, PREFS_KEY)? {
        Some(json) => serde_json::from_str::<AiPrefs>(&json).map_err(|e| format!("AI 设置解析失败: {e}"))?,
        None => migrate_legacy(db)?,
    };
    reveal_secrets(&mut prefs);
    prefs.normalize();
    Ok(prefs)
}

fn migrate_legacy(db: &Database) -> Result<AiPrefs, String> {
    let mut prefs = AiPrefs::default();
    if read_row(db, "ai_config")?.is_none() {
        return Ok(prefs);
    }
    let legacy = crate::client::load_ai_config_for_prefs(db)?;
    let provider_id = match legacy.provider.as_str() {
        "openai" => "openai",
        "ollama" => "ollama",
        _ => "custom",
    };
    let mut p = provider::new_provider(provider_id);
    p.base_url = Some(legacy.api_url).filter(|u| !u.is_empty());
    p.api_key = Some(legacy.api_key).filter(|k| !k.is_empty());
    p.default_model = Some(legacy.model.clone()).filter(|m| !m.is_empty());
    p.enabled = legacy.enabled;
    if legacy.enabled {
        prefs.active_provider_id = p.id.clone();
        prefs.active_model_id = legacy.model;
    }
    prefs.providers.push(p);
    Ok(prefs)
}

/// Writes the preferences and mirrors the active provider into the legacy `ai_config` row.
pub fn save_prefs(db: &Database, prefs: &AiPrefs) -> Result<(), String> {
    let mut prefs = prefs.clone();
    prefs.normalize();
    let sealed = seal_secrets(&prefs)?;
    let json = serde_json::to_string(&sealed).map_err(|e| e.to_string())?;
    {
        let conn = db.pool.get().map_err(|e| e.to_string())?;
        conn.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", params![PREFS_KEY, json]).map_err(|e| e.to_string())?;
    }
    sync_legacy_config(db, &prefs)
}

/// What `ai_config` (the single provider the chat clients read) should hold for these preferences.
pub fn legacy_config_for(prefs: &AiPrefs) -> Option<crate::client::AiConfig> {
    let p = prefs.active_provider().filter(|p| p.enabled)?;
    let model = if prefs.active_model_id.trim().is_empty() { p.default_model.clone().unwrap_or_default() } else { prefs.active_model_id.clone() };
    let (kind, api_url) = provider::legacy_endpoint(p);
    Some(crate::client::AiConfig { provider: kind.to_string(), api_url, api_key: p.api_key.clone().unwrap_or_default(), model, enabled: true })
}

fn sync_legacy_config(db: &Database, prefs: &AiPrefs) -> Result<(), String> {
    match legacy_config_for(prefs) {
        Some(config) => crate::client::save_ai_config_blocking(db, config),
        None => {
            // nothing active: keep the stored provider but switch the assistant off
            let Some(mut existing) = read_row(db, "ai_config")?.and_then(|j| serde_json::from_str::<crate::client::AiConfig>(&j).ok()) else { return Ok(()) };
            if !existing.enabled {
                return Ok(());
            }
            existing.enabled = false;
            let conn = db.pool.get().map_err(|e| e.to_string())?;
            let json = serde_json::to_string(&existing).map_err(|e| e.to_string())?;
            conn.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('ai_config', ?1)", params![json]).map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

/// The subset of the preferences the chat runtime consults on every request.
#[derive(Debug, Clone)]
pub struct RuntimeLimits {
    pub permission_mode: PermissionMode,
    pub command_blocklist: Vec<String>,
    pub command_timeout: u32,
    pub response_idle_timeout: u32,
    pub max_iterations: u32,
    pub permission_grants: Vec<PermissionGrantRule>,
    pub privacy: PrivacySettings,
    pub quick_messages: Vec<QuickMessage>,
    pub web_search_ready: bool,
    pub show_selection_action: bool,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        AiPrefs::default().into()
    }
}

impl From<AiPrefs> for RuntimeLimits {
    fn from(p: AiPrefs) -> Self {
        Self { permission_mode: p.global_permission_mode, command_blocklist: p.command_blocklist, command_timeout: p.command_timeout, response_idle_timeout: p.response_idle_timeout, max_iterations: p.max_iterations, permission_grants: p.permission_grants, privacy: p.privacy, show_selection_action: p.show_terminal_selection_ai_action, web_search_ready: p.web_search.as_ref().is_some_and(|w| w.is_ready()), quick_messages: p.quick_messages }
    }
}

/// Cheap read for the runtime: no secrets are fetched and a missing / unreadable document yields the defaults.
pub fn runtime_limits(db: &Database) -> RuntimeLimits {
    let Ok(Some(json)) = read_row(db, PREFS_KEY) else { return RuntimeLimits::default() };
    let mut prefs = serde_json::from_str::<AiPrefs>(&json).unwrap_or_default();
    prefs.normalize();
    prefs.into()
}

/// What the active provider adds to every request: custom headers, TLS skipping and the advanced parameters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestOptions {
    pub headers: Vec<(String, String)>,
    pub skip_tls_verify: bool,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub reasoning_effort: Option<String>,
    /// `chat` or `responses` (stored; requests still use Chat Completions).
    pub openai_api: Option<String>,
}

impl RequestOptions {
    /// The options of the provider the single-provider client talks to (the active, enabled one).
    pub fn for_provider(p: &ProviderConfig) -> Self {
        let a = p.advanced_params.clone().unwrap_or_default();
        Self {
            headers: p.custom_headers.iter().filter(|(n, _)| !n.trim().is_empty()).map(|(n, v)| (n.trim().to_string(), v.clone())).collect(),
            skip_tls_verify: p.skip_tls_verify,
            max_tokens: a.max_tokens,
            temperature: a.temperature,
            top_p: a.top_p,
            frequency_penalty: a.frequency_penalty,
            presence_penalty: a.presence_penalty,
            reasoning_effort: a.reasoning_effort,
            openai_api: p.openai_api.clone(),
        }
    }
}

/// Request options for the chat clients. Header values live in the keychain, so only those are fetched; a missing or
/// unreadable document means no extras.
pub fn request_options(db: &Database) -> RequestOptions {
    let Ok(Some(json)) = read_row(db, PREFS_KEY) else { return RequestOptions::default() };
    let Ok(prefs) = serde_json::from_str::<AiPrefs>(&json) else { return RequestOptions::default() };
    let Some(mut p) = prefs.active_provider().filter(|p| p.enabled).cloned() else { return RequestOptions::default() };
    if p.has_secret_headers {
        if let Ok(h) = ops_core::keychain::get_api_key(&provider_headers_secret_name(&p.id)) {
            p.custom_headers = serde_json::from_str(&h).unwrap_or_default();
        }
    }
    RequestOptions::for_provider(&p)
}
