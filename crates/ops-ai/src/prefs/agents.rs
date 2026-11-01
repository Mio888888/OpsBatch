//! External agent CLIs (Codex, Claude Code, Copilot, Cursor, CodeBuddy, OpenCode, Grok, MiMo): discovery on `PATH`,
//! the managed-agent entries of `externalAgents`, and the env editors of the Claude / CodeBuddy cards
//! (`managedAgentState.ts`, `claudeConfigEnv.ts`, `codebuddyConfigEnv.ts`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::AiPrefs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CodebuddyOptions {
    /// `minimal` | `low` | `medium` | `high` | `xhigh` | `max`
    pub effort: Option<String>,
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub sandbox_enabled: Option<bool>,
    pub auto_allow_bash_if_sandboxed: Option<bool>,
    pub enable_file_checkpointing: Option<bool>,
    pub fallback_model: Option<String>,
    /// Persist the session transcript to disk (default true; needs CLI >= 2.125.1).
    pub persist_session: Option<bool>,
}

impl CodebuddyOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExternalAgentConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Cursor API key (kept in the keychain).
    pub api_key: Option<String>,
    pub icon: Option<String>,
    pub enabled: bool,
    pub available: Option<bool>,
    pub sdk_backend: Option<String>,
    /// Cursor only: `api-key` or `cli-login`.
    pub cursor_auth_mode: Option<String>,
    /// Codex transport: `sdk` (default) or `app-server`.
    pub codex_runtime: Option<String>,
    /// Grok transport: `acp` (default) or `streaming-json`.
    pub grok_runtime: Option<String>,
    /// `manual` when the user typed the path, `auto` when it came from `PATH`.
    pub command_source: Option<String>,
    pub cli_version: Option<String>,
    pub codebuddy_options: Option<CodebuddyOptions>,
}

pub struct ManagedAgent {
    pub key: &'static str,
    pub name: &'static str,
    /// Binary names looked up on `PATH`, in order.
    pub commands: &'static [&'static str],
    pub args: &'static [&'static str],
    pub icon: &'static str,
    /// Env key the discovered binary path is stored under.
    pub path_env: Option<&'static str>,
}

pub const MANAGED_AGENTS: &[ManagedAgent] = &[
    ManagedAgent {
        key: "codex",
        name: "Codex CLI",
        commands: &["codex"],
        args: &["exec", "--full-auto", "--json", "{prompt}"],
        icon: "openai",
        path_env: None,
    },
    ManagedAgent {
        key: "claude",
        name: "Claude Code",
        commands: &["claude"],
        args: &["-p", "--output-format", "text", "{prompt}"],
        icon: "claude",
        path_env: Some("CLAUDE_CODE_EXECUTABLE"),
    },
    ManagedAgent {
        key: "copilot",
        name: "GitHub Copilot CLI",
        commands: &["copilot"],
        args: &["-p", "{prompt}"],
        icon: "copilot",
        path_env: None,
    },
    ManagedAgent {
        key: "cursor",
        name: "Cursor",
        commands: &["cursor-agent", "cursor"],
        args: &["{prompt}"],
        icon: "cursor",
        path_env: None,
    },
    ManagedAgent {
        key: "codebuddy",
        name: "CodeBuddy Code",
        commands: &["codebuddy"],
        args: &[],
        icon: "codebuddy",
        path_env: Some("CODEBUDDY_CODE_PATH"),
    },
    ManagedAgent {
        key: "opencode",
        name: "OpenCode",
        commands: &["opencode"],
        args: &[],
        icon: "opencode",
        path_env: Some("OPENCODE_BIN"),
    },
    ManagedAgent {
        key: "grok",
        name: "Grok Build",
        commands: &["grok"],
        args: &[],
        icon: "grok",
        path_env: None,
    },
    ManagedAgent {
        key: "mimo",
        name: "MiMo Code",
        commands: &["mimo"],
        args: &[],
        icon: "mimo",
        path_env: Some("MIMOCODE_BIN"),
    },
];

pub fn managed_agent(key: &str) -> Option<&'static ManagedAgent> {
    MANAGED_AGENTS.iter().find(|a| a.key == key)
}

pub fn managed_id(key: &str) -> String {
    format!("discovered_{key}")
}

/// What a probe of an agent binary found (`AgentPathInfo`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentPathInfo {
    pub path: Option<String>,
    pub version: Option<String>,
    pub available: bool,
}

/// `isPathLikeCommand`: contains a separator or a drive prefix.
pub fn is_path_like(command: &str) -> bool {
    command.contains('/')
        || command.contains('\\')
        || (command.len() > 1
            && command.as_bytes()[1] == b':'
            && command.as_bytes()[0].is_ascii_alphabetic())
}

/// Directories a GUI app usually misses on its `PATH` (the Dock does not source the shell profile).
fn extra_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    {
        for sub in [
            ".local/bin",
            ".npm-global/bin",
            ".cargo/bin",
            ".bun/bin",
            ".volta/bin",
            ".claude/local",
            ".codeium/windsurf/bin",
            "bin",
        ] {
            dirs.push(home.join(sub));
        }
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// First executable called `command` in `PATH`, then in the usual install directories.
pub fn find_in_path(command: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(extra_search_dirs());
    let names: Vec<String> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ext| format!("{command}{ext}"))
            .collect()
    } else {
        vec![command.to_string()]
    };
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| is_executable(p))
}

/// `extractCliSemver`: the first `major.minor.patch` in the `--version` output.
pub fn extract_cli_semver(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"(\d+)\.(\d+)\.(\d+)").ok()?;
    let c = re.captures(text)?;
    Some(format!("{}.{}.{}", &c[1], &c[2], &c[3]))
}

pub fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (pa, pb) = (parse(a), parse(b));
    (0..pa.len().max(pb.len()))
        .map(|i| pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

pub const CODEBUDDY_PERSIST_SESSION_MIN_CLI_VERSION: &str = "2.125.1";

/// `isCodebuddySessionPersistenceSupported`
pub fn codebuddy_persistence_supported(version_text: Option<&str>) -> bool {
    version_text
        .and_then(extract_cli_semver)
        .is_some_and(|v| compare_semver(&v, CODEBUDDY_PERSIST_SESSION_MIN_CLI_VERSION).is_ge())
}

/// Runs `<binary> --version` and returns the first output line (a CLI that does not answer in time is still found).
pub async fn probe_version(binary: &Path) -> Option<String> {
    let mut child = tokio::process::Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let read = async {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        let _ = child.wait().await;
        buf
    };
    let out = tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .ok()?;
    String::from_utf8_lossy(&out)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// Resolves the binary for an agent: the user's path when given, else the first command found on `PATH`.
pub async fn resolve_cli(key: &str, custom_path: &str) -> AgentPathInfo {
    let custom = custom_path.trim();
    let binary = if !custom.is_empty() {
        let p = PathBuf::from(custom);
        if is_executable(&p) {
            Some(p)
        } else {
            find_in_path(custom).filter(|_| !is_path_like(custom))
        }
    } else {
        managed_agent(key).and_then(|a| a.commands.iter().find_map(|c| find_in_path(c)))
    };
    match binary {
        Some(path) => {
            let version = probe_version(&path).await;
            AgentPathInfo {
                path: Some(path.to_string_lossy().into_owned()),
                version,
                available: true,
            }
        }
        None => AgentPathInfo::default(),
    }
}

fn defaults_for(key: &str) -> ExternalAgentConfig {
    let a = managed_agent(key).expect("known agent");
    ExternalAgentConfig {
        name: a.name.into(),
        args: a.args.iter().map(|s| s.to_string()).collect(),
        icon: Some(a.icon.into()),
        sdk_backend: Some(a.key.into()),
        ..Default::default()
    }
}

/// `buildManagedAgentState`: folds a probe result into the `externalAgents` list (and the default agent).
pub fn apply_probe(prefs: &mut AiPrefs, key: &str, info: &AgentPathInfo, source: &str) {
    let Some(agent) = managed_agent(key) else {
        return;
    };
    let id = managed_id(key);
    let existing = prefs.external_agents.iter().find(|a| a.id == id).cloned();
    let others: Vec<ExternalAgentConfig> = prefs
        .external_agents
        .iter()
        .filter(|a| a.id != id)
        .cloned()
        .collect();
    let was_default = prefs.default_agent_id == id;

    let Some(path) = info.path.clone().filter(|_| info.available) else {
        let keep_cursor = key == "cursor"
            && existing.as_ref().is_some_and(|e| {
                e.api_key.is_some() || e.cursor_auth_mode.as_deref() == Some("cli-login")
            });
        let keep_claude = key == "claude"
            && existing
                .as_ref()
                .is_some_and(|e| e.env.keys().any(|k| k != "CLAUDE_CODE_EXECUTABLE"));
        let keep_codebuddy = key == "codebuddy"
            && existing.as_ref().is_some_and(|e| {
                !e.env.is_empty() || e.codebuddy_options.as_ref().is_some_and(|o| !o.is_empty())
            });
        prefs.external_agents = others;
        if let (true, Some(mut e)) = (keep_cursor || keep_codebuddy || keep_claude, existing) {
            let defaults = defaults_for(key);
            e.name = defaults.name;
            e.args = defaults.args;
            e.icon = defaults.icon;
            e.sdk_backend = defaults.sdk_backend;
            if e.command.is_empty() {
                e.command = agent.commands[0].to_string();
            }
            e.available = Some(false);
            if keep_codebuddy || keep_claude {
                e.enabled = false;
            } else {
                e.enabled = true;
                if e.cursor_auth_mode.as_deref() != Some("cli-login") {
                    e.cursor_auth_mode = Some("api-key".into());
                }
            }
            prefs.external_agents.push(e);
        }
        if was_default {
            prefs.default_agent_id = "catty".into();
        }
        return;
    };

    let mut next = existing.clone().unwrap_or_else(|| ExternalAgentConfig {
        id: id.clone(),
        ..defaults_for(key)
    });
    let defaults = defaults_for(key);
    next.id = id.clone();
    next.name = defaults.name;
    next.args = defaults.args;
    next.icon = defaults.icon;
    next.sdk_backend = defaults.sdk_backend;
    next.command = path.clone();
    next.command_source = Some(source.to_string());
    if let Some(v) = &info.version {
        next.cli_version = Some(v.clone());
    }
    if let Some(env_key) = agent.path_env {
        next.env.insert(env_key.to_string(), path);
    }
    next.available = Some(true);
    next.enabled = existing.as_ref().map(|e| e.enabled).unwrap_or(true);
    if key == "cursor" && next.cursor_auth_mode.is_none() {
        next.cursor_auth_mode = Some("cli-login".into());
    }
    prefs.external_agents = others;
    prefs.external_agents.push(next);
    prefs.external_agents.sort_by_key(|a| {
        MANAGED_AGENTS
            .iter()
            .position(|m| managed_id(m.key) == a.id)
            .unwrap_or(usize::MAX)
    });
}

/// The default-agent choices: the built-in assistant plus every enabled, available agent.
pub fn agent_options(prefs: &AiPrefs) -> Vec<(String, String)> {
    let mut options = vec![("catty".to_string(), "Catty".to_string())];
    options.extend(
        prefs
            .external_agents
            .iter()
            .filter(|a| a.enabled && a.available != Some(false))
            .map(|a| (a.id.clone(), a.name.clone())),
    );
    options
}

// ---------------------------------------------------------------------------
// Codex login
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexState {
    ConnectedChatGpt,
    ConnectedApiKey,
    ConnectedCustomConfig,
    NotLoggedIn,
    Unknown,
}

/// The custom `model_provider` of `~/.codex/config.toml`, which makes ChatGPT login unnecessary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexCustomConfig {
    pub provider_name: String,
    pub display_name: String,
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub env_key_present: bool,
    pub has_hardcoded_api_key: bool,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexStatus {
    pub state: CodexState,
    pub raw_output: String,
    pub exit_code: Option<i32>,
    pub custom_config: Option<CodexCustomConfig>,
}

impl CodexStatus {
    pub fn is_connected(&self) -> bool {
        matches!(
            self.state,
            CodexState::ConnectedChatGpt
                | CodexState::ConnectedApiKey
                | CodexState::ConnectedCustomConfig
        )
    }
}

/// `normalizeCodexIntegrationState`
pub fn classify_codex_output(output: &str) -> CodexState {
    let o = output.to_lowercase();
    if o.contains("logged in using chatgpt") {
        CodexState::ConnectedChatGpt
    } else if o.contains("logged in using an api key") || o.contains("logged in using api key") {
        CodexState::ConnectedApiKey
    } else if o.contains("not logged in") {
        CodexState::NotLoggedIn
    } else {
        CodexState::Unknown
    }
}

fn toml_value(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once('=')?;
    let v = v.trim();
    let v = v
        .strip_prefix('"')
        .and_then(|x| x.rsplit_once('"').map(|(a, _)| a))
        .unwrap_or(v);
    Some((k.trim().to_string(), v.to_string()))
}

/// Reads the custom provider out of a `config.toml` text. `env` looks an environment variable up.
pub fn parse_codex_config(
    toml: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Option<CodexCustomConfig> {
    let mut top: BTreeMap<String, String> = BTreeMap::new();
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for raw in toml.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(name.trim().to_string());
            continue;
        }
        if let Some((k, v)) = toml_value(line) {
            match &current {
                None => {
                    top.insert(k, v);
                }
                Some(sec) => {
                    sections.entry(sec.clone()).or_default().insert(k, v);
                }
            }
        }
    }
    let provider = top.get("model_provider")?.clone();
    // the built-in provider still goes through ChatGPT / API-key auth
    if provider.is_empty() || provider == "openai" {
        return None;
    }
    let table = sections
        .get(&format!("model_providers.{provider}"))
        .cloned()
        .unwrap_or_default();
    let env_key = table.get("env_key").cloned().filter(|k| !k.is_empty());
    Some(CodexCustomConfig {
        display_name: table
            .get("name")
            .cloned()
            .unwrap_or_else(|| provider.clone()),
        provider_name: provider,
        base_url: table.get("base_url").cloned().filter(|u| !u.is_empty()),
        env_key_present: env_key
            .as_deref()
            .is_some_and(|k| env(k).is_some_and(|v| !v.is_empty())),
        env_key,
        has_hardcoded_api_key: table
            .keys()
            .any(|k| k == "experimental_bearer_token" || k == "api_key"),
        model: top.get("model").cloned(),
    })
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| PathBuf::from(h).join(".codex"))
        })
}

pub fn read_codex_custom_config() -> Option<CodexCustomConfig> {
    let text = std::fs::read_to_string(codex_home()?.join("config.toml")).ok()?;
    parse_codex_config(&text, |k| std::env::var(k).ok())
}

/// Folds the CLI output and the custom config into one status (a custom provider wins over "not logged in").
pub fn codex_status_from(
    output: &str,
    exit_code: Option<i32>,
    custom: Option<CodexCustomConfig>,
) -> CodexStatus {
    let mut state = classify_codex_output(output);
    if custom.is_some() && matches!(state, CodexState::NotLoggedIn | CodexState::Unknown) {
        state = CodexState::ConnectedCustomConfig;
    }
    CodexStatus {
        state,
        raw_output: output.trim().to_string(),
        exit_code,
        custom_config: custom,
    }
}

async fn run_cli(binary: &Path, args: &[&str]) -> Result<(String, Option<i32>), String> {
    let child = tokio::process::Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let out = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| "命令超时".to_string())?
        .map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((text, out.status.code()))
}

/// `codex login status`
pub async fn codex_login_status(binary: &Path) -> CodexStatus {
    let custom = read_codex_custom_config();
    match run_cli(binary, &["login", "status"]).await {
        Ok((out, code)) => codex_status_from(&out, code, custom),
        Err(e) => codex_status_from(&e, None, custom),
    }
}

/// `codex logout`; returns the refreshed status.
pub async fn codex_logout(binary: &Path) -> CodexStatus {
    let _ = run_cli(binary, &["logout"]).await;
    codex_login_status(binary).await
}

/// First `https://` URL in a line of CLI output.
pub fn find_login_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    Some(
        text[start..]
            .split(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .next()?
            .trim_end_matches(['.', ',', ')'])
            .to_string(),
    )
}

/// A running `codex login` (the browser flow): the URL it printed and a way to stop it.
pub struct CodexLogin {
    pub url: Option<String>,
    pub output: String,
    child: tokio::process::Child,
}

impl CodexLogin {
    pub async fn cancel(mut self) {
        let _ = self.child.kill().await;
    }

    /// `Some(exit code)` once the login process ended.
    pub fn finished(&mut self) -> Option<Option<i32>> {
        self.child.try_wait().ok().flatten().map(|s| s.code())
    }
}

/// Starts `codex login` and waits (up to 15 s) for the URL it prints.
pub async fn codex_login_start(binary: &Path) -> Result<CodexLogin, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut child = tokio::process::Command::new(binary)
        .arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("no stdout")?).lines();
    let mut err_lines = BufReader::new(child.stderr.take().ok_or("no stderr")?).lines();
    let mut output = String::new();
    let mut url = None;
    let wait = async {
        loop {
            let line = tokio::select! {
                l = lines.next_line() => l,
                l = err_lines.next_line() => l,
            };
            match line {
                Ok(Some(l)) => {
                    output.push_str(&l);
                    output.push('\n');
                    if let Some(u) = find_login_url(&l) {
                        url = Some(u);
                        break;
                    }
                }
                _ => break,
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(15), wait).await;
    Ok(CodexLogin { url, output, child })
}

// ---------------------------------------------------------------------------
// Env editors
// ---------------------------------------------------------------------------

/// `KEY=value` lines; blank lines, `#` comments and lines without a key are skipped.
pub fn parse_env_lines(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=').filter(|i| *i > 0) else {
            continue;
        };
        let key = line[..eq].trim();
        if !key.is_empty() {
            out.insert(key.to_string(), line[eq + 1..].trim().to_string());
        }
    }
    out
}

pub fn serialize_env_lines(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

const CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";
const CLAUDE_SETTINGS: &str = "OPSBATCH_CLAUDE_SETTINGS";
const CLAUDE_MANAGED: [&str; 3] = ["CLAUDE_CODE_EXECUTABLE", CLAUDE_CONFIG_DIR, CLAUDE_SETTINGS];
const CODEBUDDY_INTERNET: &str = "CODEBUDDY_INTERNET_ENVIRONMENT";
const CODEBUDDY_MANAGED: [&str; 2] = [CODEBUDDY_INTERNET, "CODEBUDDY_CODE_PATH"];

/// `(config dir, settings path, free-text env)` of a Claude entry.
pub fn split_claude_env(env: &BTreeMap<String, String>) -> (String, String, String) {
    let rest: BTreeMap<String, String> = env
        .iter()
        .filter(|(k, _)| !CLAUDE_MANAGED.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (
        env.get(CLAUDE_CONFIG_DIR).cloned().unwrap_or_default(),
        env.get(CLAUDE_SETTINGS).cloned().unwrap_or_default(),
        serialize_env_lines(&rest),
    )
}

pub fn build_claude_env(
    prev: &BTreeMap<String, String>,
    config_dir: &str,
    settings_path: &str,
    env_text: &str,
) -> BTreeMap<String, String> {
    let mut next = BTreeMap::new();
    if let Some(exe) = prev.get("CLAUDE_CODE_EXECUTABLE") {
        next.insert("CLAUDE_CODE_EXECUTABLE".to_string(), exe.clone());
    }
    if !config_dir.trim().is_empty() {
        next.insert(CLAUDE_CONFIG_DIR.into(), config_dir.trim().into());
    }
    if !settings_path.trim().is_empty() {
        next.insert(CLAUDE_SETTINGS.into(), settings_path.trim().into());
    }
    let mut parsed = parse_env_lines(env_text);
    for key in CLAUDE_MANAGED {
        parsed.remove(key);
    }
    next.extend(parsed);
    next
}

/// `(internet environment, free-text env)` of a CodeBuddy entry.
pub fn split_codebuddy_env(env: &BTreeMap<String, String>) -> (String, String) {
    let rest: BTreeMap<String, String> = env
        .iter()
        .filter(|(k, _)| !CODEBUDDY_MANAGED.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (
        env.get(CODEBUDDY_INTERNET).cloned().unwrap_or_default(),
        serialize_env_lines(&rest),
    )
}

pub fn build_codebuddy_env(
    prev: &BTreeMap<String, String>,
    internet_env: &str,
    env_text: &str,
) -> BTreeMap<String, String> {
    let mut next = BTreeMap::new();
    if !internet_env.trim().is_empty() {
        next.insert(CODEBUDDY_INTERNET.into(), internet_env.trim().into());
    }
    if let Some(path) = prev.get("CODEBUDDY_CODE_PATH") {
        next.insert("CODEBUDDY_CODE_PATH".to_string(), path.clone());
    }
    let mut parsed = parse_env_lines(env_text);
    for key in CODEBUDDY_MANAGED {
        parsed.remove(key);
    }
    next.extend(parsed);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str) -> AgentPathInfo {
        AgentPathInfo {
            path: Some(path.into()),
            version: Some("1.2.3".into()),
            available: true,
        }
    }

    #[test]
    fn env_lines_round_trip() {
        let env =
            parse_env_lines("# comment\nA=1\n\n B = two words \n=bad\nnoequals\nURL=http://x?a=b");
        assert_eq!(env.len(), 3);
        assert_eq!(env["B"], "two words");
        assert_eq!(env["URL"], "http://x?a=b");
        assert_eq!(parse_env_lines(&serialize_env_lines(&env)), env);
    }

    #[test]
    fn claude_env_keeps_managed_keys_out_of_the_free_text() {
        let prev = BTreeMap::from([(
            "CLAUDE_CODE_EXECUTABLE".to_string(),
            "/bin/claude".to_string(),
        )]);
        let env = build_claude_env(
            &prev,
            " /cfg ",
            "/s.json",
            "FOO=1\nCLAUDE_CONFIG_DIR=evil\nCLAUDE_CODE_EXECUTABLE=x",
        );
        assert_eq!(env["CLAUDE_CODE_EXECUTABLE"], "/bin/claude");
        assert_eq!(env["CLAUDE_CONFIG_DIR"], "/cfg");
        assert_eq!(env["OPSBATCH_CLAUDE_SETTINGS"], "/s.json");
        assert_eq!(env["FOO"], "1");
        let (dir, settings, text) = split_claude_env(&env);
        assert_eq!(
            (dir.as_str(), settings.as_str(), text.as_str()),
            ("/cfg", "/s.json", "FOO=1")
        );
        assert!(build_claude_env(&BTreeMap::new(), "", "", "").is_empty());
    }

    #[test]
    fn codebuddy_env_preserves_the_discovered_path() {
        let prev = BTreeMap::from([("CODEBUDDY_CODE_PATH".to_string(), "/bin/cb".to_string())]);
        let env = build_codebuddy_env(
            &prev,
            "internal",
            "CODEBUDDY_API_KEY=k\nCODEBUDDY_INTERNET_ENVIRONMENT=nope",
        );
        assert_eq!(env["CODEBUDDY_INTERNET_ENVIRONMENT"], "internal");
        assert_eq!(env["CODEBUDDY_CODE_PATH"], "/bin/cb");
        assert_eq!(
            split_codebuddy_env(&env),
            ("internal".to_string(), "CODEBUDDY_API_KEY=k".to_string())
        );
    }

    #[test]
    fn versions_are_extracted_and_compared() {
        assert_eq!(
            extract_cli_semver("codex-cli 0.144.1 (abc)").as_deref(),
            Some("0.144.1")
        );
        assert_eq!(extract_cli_semver("no version"), None);
        assert!(compare_semver("2.125.1", "2.125.1").is_eq());
        assert!(compare_semver("2.126.0", "2.125.9").is_gt());
        assert!(compare_semver("1.9.0", "1.10.0").is_lt());
        assert!(codebuddy_persistence_supported(Some("CodeBuddy 2.130.0")));
        assert!(!codebuddy_persistence_supported(Some("2.100.0")));
        assert!(!codebuddy_persistence_supported(None));
    }

    #[test]
    fn path_like_commands() {
        assert!(is_path_like("/usr/bin/claude"));
        assert!(is_path_like("C:\\x\\claude.cmd"));
        assert!(is_path_like("./claude"));
        assert!(!is_path_like("claude"));
    }

    #[test]
    fn a_found_agent_is_added_enabled_and_keeps_user_choices() {
        let mut prefs = AiPrefs::default();
        apply_probe(&mut prefs, "claude", &found("/bin/claude"), "auto");
        let a = &prefs.external_agents[0];
        assert_eq!(
            (a.id.as_str(), a.command.as_str(), a.enabled, a.available),
            ("discovered_claude", "/bin/claude", true, Some(true))
        );
        assert_eq!(a.env["CLAUDE_CODE_EXECUTABLE"], "/bin/claude");
        assert_eq!(a.cli_version.as_deref(), Some("1.2.3"));
        assert_eq!(a.sdk_backend.as_deref(), Some("claude"));
        prefs.external_agents[0].enabled = false;
        apply_probe(&mut prefs, "claude", &found("/opt/claude"), "manual");
        assert!(!prefs.external_agents[0].enabled);
        assert_eq!(
            prefs.external_agents[0].command_source.as_deref(),
            Some("manual")
        );
        assert_eq!(prefs.external_agents.len(), 1);
    }

    #[test]
    fn a_missing_agent_is_removed_and_the_default_falls_back() {
        let mut prefs = AiPrefs::default();
        apply_probe(&mut prefs, "codex", &found("/bin/codex"), "auto");
        prefs.default_agent_id = "discovered_codex".into();
        apply_probe(&mut prefs, "codex", &AgentPathInfo::default(), "auto");
        assert!(prefs.external_agents.is_empty());
        assert_eq!(prefs.default_agent_id, "catty");
    }

    #[test]
    fn cursor_with_a_stored_key_survives_a_missing_cli() {
        let mut prefs = AiPrefs::default();
        prefs.external_agents.push(ExternalAgentConfig {
            id: "discovered_cursor".into(),
            command: "cursor-agent".into(),
            api_key: Some("k".into()),
            enabled: true,
            ..Default::default()
        });
        apply_probe(&mut prefs, "cursor", &AgentPathInfo::default(), "auto");
        let a = &prefs.external_agents[0];
        assert_eq!(
            (a.available, a.enabled, a.cursor_auth_mode.as_deref()),
            (Some(false), true, Some("api-key"))
        );
    }

    #[test]
    fn default_agent_choices_list_enabled_available_agents() {
        let mut prefs = AiPrefs::default();
        apply_probe(&mut prefs, "codex", &found("/bin/codex"), "auto");
        apply_probe(&mut prefs, "grok", &found("/bin/grok"), "auto");
        prefs.external_agents[1].enabled = false;
        let ids: Vec<String> = agent_options(&prefs)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["catty", "discovered_codex"]);
    }

    #[test]
    fn codex_status_output_is_classified() {
        assert_eq!(
            classify_codex_output("Logged in using ChatGPT"),
            CodexState::ConnectedChatGpt
        );
        assert_eq!(
            classify_codex_output("Logged in using an API key - sk-***"),
            CodexState::ConnectedApiKey
        );
        assert_eq!(
            classify_codex_output("Not logged in"),
            CodexState::NotLoggedIn
        );
        assert_eq!(classify_codex_output("???"), CodexState::Unknown);
    }

    #[test]
    fn a_custom_codex_provider_counts_as_connected() {
        let toml = "model = \"gpt-x\"\nmodel_provider = \"azure\"\n\n[model_providers.azure]\nname = \"Azure OpenAI\"\nbase_url = \"https://az.test/v1\"\nenv_key = \"AZ_KEY\"\n";
        let with_env =
            parse_codex_config(toml, |k| (k == "AZ_KEY").then(|| "secret".to_string())).unwrap();
        assert_eq!(
            (
                with_env.provider_name.as_str(),
                with_env.display_name.as_str(),
                with_env.base_url.as_deref(),
                with_env.env_key_present,
                with_env.model.as_deref()
            ),
            (
                "azure",
                "Azure OpenAI",
                Some("https://az.test/v1"),
                true,
                Some("gpt-x")
            )
        );
        let without = parse_codex_config(toml, |_| None).unwrap();
        assert!(!without.env_key_present);
        assert_eq!(
            codex_status_from("Not logged in", Some(1), Some(without)).state,
            CodexState::ConnectedCustomConfig
        );
        assert_eq!(
            codex_status_from("Not logged in", Some(1), None).state,
            CodexState::NotLoggedIn
        );
        assert!(parse_codex_config("model_provider = \"openai\"", |_| None).is_none());
        assert!(parse_codex_config("model = \"x\"", |_| None).is_none());
        assert!(codex_status_from("Logged in using ChatGPT", Some(0), None).is_connected());
    }

    #[test]
    fn the_login_url_is_found_in_cli_output() {
        assert_eq!(
            find_login_url("Open https://auth.openai.com/oauth?x=1&y=2 to continue."),
            Some("https://auth.openai.com/oauth?x=1&y=2".to_string())
        );
        assert_eq!(find_login_url("nothing here"), None);
    }

    #[tokio::test]
    async fn resolve_cli_finds_a_real_binary_and_rejects_a_missing_one() {
        let this_test = std::env::current_exe().unwrap();
        let this_test = this_test.to_str().unwrap();
        let info = resolve_cli("codex", this_test).await;
        assert!(info.available);
        assert_eq!(info.path.as_deref(), Some(this_test));
        assert!(!resolve_cli("codex", "/definitely/not/here").await.available);
    }
}
