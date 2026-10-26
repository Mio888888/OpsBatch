//! External MCP settings and the client-setup snippets of Netcatty's "External MCP" card: the bridge that lets an
//! outside agent (Codex, Claude Code, Grok, Cursor, ...) drive the app's terminal sessions.

use serde::{Deserialize, Serialize};

pub const DISCOVERY_ENV_VAR: &str = "OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE";
pub const HTTP_BEARER_ENV: &str = "OPSBATCH_MCP_BEARER_TOKEN";
pub const DEFAULT_HTTP_PORT: u16 = 47391;
pub const DEFAULT_HTTP_HOST: &str = "127.0.0.1";
pub const HTTP_PATH: &str = "/mcp";
pub const SERVER_NAME: &str = "opsbatch-external";
pub const HTTP_SERVER_NAME: &str = "opsbatch-http";
pub const DEFAULT_IDLE_TIMEOUT_MINUTES: u32 = 10;
pub const DEFAULT_SESSION_IDLE_TIMEOUT_MINUTES: u32 = 480;
const MIN_MINUTES: u32 = 1;
const MAX_MINUTES: u32 = 24 * 60;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Temporary,
    #[default]
    Persistent,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temporary => "temporary",
            Self::Persistent => "persistent",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalAlert {
    Off,
    #[default]
    Unfocused,
    Always,
}

impl ApprovalAlert {
    pub const ALL: [ApprovalAlert; 3] = [Self::Off, Self::Unfocused, Self::Always];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Unfocused => "unfocused",
            Self::Always => "always",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExternalMcpHttpSettings {
    pub enabled: bool,
    pub port: u16,
    pub allow_custom_bind: bool,
    pub listen_address: String,
    pub client_host: String,
    /// Bearer token of the HTTP transport (kept in the keychain; only filled in memory).
    #[serde(skip)]
    pub token: String,
}

impl Default for ExternalMcpHttpSettings {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_HTTP_PORT, allow_custom_bind: false, listen_address: DEFAULT_HTTP_HOST.into(), client_host: String::new(), token: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExternalMcpSettings {
    pub enabled: bool,
    pub mode: Mode,
    pub idle_timeout_minutes: u32,
    pub session_idle_timeout_minutes: u32,
    pub silent_sessions: bool,
    pub focus_on_host_open: bool,
    pub approval_alert: ApprovalAlert,
    pub http: ExternalMcpHttpSettings,
}

impl Default for ExternalMcpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::Persistent,
            idle_timeout_minutes: DEFAULT_IDLE_TIMEOUT_MINUTES,
            session_idle_timeout_minutes: DEFAULT_SESSION_IDLE_TIMEOUT_MINUTES,
            silent_sessions: false,
            focus_on_host_open: false,
            approval_alert: ApprovalAlert::Unfocused,
            http: ExternalMcpHttpSettings::default(),
        }
    }
}

impl ExternalMcpSettings {
    pub fn normalize(&mut self) {
        self.idle_timeout_minutes = self.idle_timeout_minutes.clamp(MIN_MINUTES, MAX_MINUTES);
        self.session_idle_timeout_minutes = self.session_idle_timeout_minutes.clamp(MIN_MINUTES, MAX_MINUTES);
        if self.http.port < 1024 {
            self.http.port = DEFAULT_HTTP_PORT;
        }
        if self.http.listen_address.trim().is_empty() {
            self.http.listen_address = DEFAULT_HTTP_HOST.into();
        }
        // a custom bind address only counts while the switch allows it
        if !self.http.allow_custom_bind {
            self.http.listen_address = DEFAULT_HTTP_HOST.into();
        }
    }
}

pub fn is_wildcard_address(address: &str) -> bool {
    matches!(address.trim(), "0.0.0.0" | "::")
}

/// Listening on loopback keeps the bridge reachable from this machine only.
pub fn is_loopback_or_default(address: &str) -> bool {
    matches!(address.trim(), "127.0.0.1" | "::1" | "localhost" | "")
}

pub fn is_ip_address(value: &str) -> bool {
    let v = value.trim();
    if let Some(parts) = Some(v.split('.').collect::<Vec<_>>()).filter(|p| p.len() == 4) {
        return parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255) && p.parse::<u16>().map(|n| n.to_string()) == Ok((*p).to_string()));
    }
    v.contains(':') && v.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
}

/// `1024..=65535`
pub fn parse_port(value: &str) -> Option<u16> {
    value.trim().parse::<u32>().ok().filter(|p| (1024..=65535).contains(p)).map(|p| p as u16)
}

fn format_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() }
}

/// URL the client connects to; a wildcard bind needs the host the client reaches this machine by.
pub fn http_url(listen_address: &str, port: u16, client_host: &str) -> String {
    let host = if is_wildcard_address(listen_address) { client_host.trim() } else { listen_address.trim() };
    if host.is_empty() || is_wildcard_address(host) {
        return String::new();
    }
    format!("http://{}:{port}{HTTP_PATH}", format_host(host))
}

/// A random 256-bit token, hex encoded.
pub fn generate_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn codex_http_toml(url: &str) -> String {
    format!("[mcp_servers.{HTTP_SERVER_NAME}]\nurl = {}\nbearer_token_env_var = {}", serde_json::to_string(url).unwrap(), serde_json::to_string(HTTP_BEARER_ENV).unwrap())
}

pub fn codex_http_env_line(token: &str) -> String {
    format!("export {HTTP_BEARER_ENV}={}", shell_quote(token))
}

pub fn claude_http_add_command(url: &str, token: &str) -> String {
    ["claude mcp add --transport http", HTTP_SERVER_NAME, &shell_quote(url), "--header", &shell_quote(&format!("Authorization: Bearer {token}"))].join(" ")
}

pub fn generic_http_json(url: &str, token: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({"mcpServers": {HTTP_SERVER_NAME: {"type": "http", "url": url, "headers": {"Authorization": format!("Bearer {token}")}}}})).unwrap()
}

fn env_flag(flag: &str, discovery: Option<&str>) -> String {
    discovery.map(|d| format!(" {flag} {DISCOVERY_ENV_VAR}={}", shell_quote(d))).unwrap_or_default()
}

pub fn codex_add_command(launcher: &str, discovery: Option<&str>) -> String {
    format!("codex mcp add {SERVER_NAME}{} -- {}", env_flag("--env", discovery), shell_quote(launcher))
}

pub fn claude_add_command(launcher: &str, discovery: Option<&str>) -> String {
    format!("claude mcp add -s user {SERVER_NAME}{} -- {}", env_flag("-e", discovery), shell_quote(launcher))
}

pub fn grok_add_command(launcher: &str, discovery: Option<&str>) -> String {
    format!("grok mcp add {SERVER_NAME}{} -- {}", env_flag("-e", discovery), shell_quote(launcher))
}

/// TOML table shared by Codex and Grok.
pub fn toml_snippet(launcher: &str, discovery: Option<&str>) -> String {
    let env = discovery.map(|d| format!("\nenv = {{ {DISCOVERY_ENV_VAR} = {} }}", toml_string(d))).unwrap_or_default();
    format!("[mcp_servers.{SERVER_NAME}]\ncommand = {}\nargs = []{env}", toml_string(launcher))
}

/// JSON `mcpServers` entry shared by Claude Code and Cursor.
pub fn json_snippet(launcher: &str, discovery: Option<&str>) -> String {
    let mut entry = serde_json::json!({"command": launcher, "args": []});
    if let Some(d) = discovery {
        entry["env"] = serde_json::json!({ DISCOVERY_ENV_VAR: d });
    }
    serde_json::to_string_pretty(&serde_json::json!({"mcpServers": {SERVER_NAME: entry}})).unwrap()
}

/// Prompt that lets an outside AI client register the server itself (`buildMcpOnboardingPrompt`).
pub fn onboarding_prompt(launcher: Option<&str>, discovery: Option<&str>) -> String {
    let Some(launcher) = launcher.filter(|l| !l.is_empty()) else {
        return "Please connect OpsBatch to this session via MCP. In the OpsBatch desktop app, open Settings → AI → Tool Access, turn on External MCP, then copy the generated prompt from the Tool Access section and run it here. After that, list the opsbatch-external MCP tools and call get_environment to verify the connection.".to_string();
    };
    let mut lines = vec!["Please register OpsBatch's MCP server in your MCP client configuration:".to_string(), format!("- Server name: {SERVER_NAME}"), "- Transport: local stdio".to_string(), format!("- Command: {launcher}")];
    if let Some(d) = discovery.filter(|d| !d.is_empty()) {
        lines.push(format!("- Environment: {DISCOVERY_ENV_VAR}={d}"));
    }
    lines.push("After registering, list the server's tools and call get_environment to verify the connection.".into());
    lines.push("Keep the OpsBatch desktop app running while you use these tools.".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_wrap_ipv6_and_need_a_client_host_for_wildcards() {
        assert_eq!(http_url("127.0.0.1", 47391, ""), "http://127.0.0.1:47391/mcp");
        assert_eq!(http_url("::1", 4000, ""), "http://[::1]:4000/mcp");
        assert_eq!(http_url("0.0.0.0", 4000, ""), "");
        assert_eq!(http_url("0.0.0.0", 4000, " 10.0.0.5 "), "http://10.0.0.5:4000/mcp");
        assert_eq!(http_url("::", 4000, "0.0.0.0"), "");
    }

    #[test]
    fn ports_and_addresses_are_validated() {
        assert_eq!(parse_port("47391"), Some(47391));
        assert_eq!(parse_port("80"), None);
        assert_eq!(parse_port("70000"), None);
        assert_eq!(parse_port("x"), None);
        assert!(is_ip_address("192.168.1.10"));
        assert!(!is_ip_address("256.1.1.1"));
        assert!(!is_ip_address("01.2.3.4"));
        assert!(is_ip_address("::1"));
        assert!(!is_ip_address("host.local"));
        assert!(is_wildcard_address(" 0.0.0.0 "));
    }

    #[test]
    fn settings_are_clamped_and_loopback_unless_custom_bind_is_on() {
        let mut s = ExternalMcpSettings { idle_timeout_minutes: 0, session_idle_timeout_minutes: 99999, ..Default::default() };
        s.http.port = 80;
        s.http.listen_address = "0.0.0.0".into();
        s.normalize();
        assert_eq!((s.idle_timeout_minutes, s.session_idle_timeout_minutes, s.http.port), (1, 1440, DEFAULT_HTTP_PORT));
        assert_eq!(s.http.listen_address, "127.0.0.1");
        s.http.allow_custom_bind = true;
        s.http.listen_address = "0.0.0.0".into();
        s.normalize();
        assert_eq!(s.http.listen_address, "0.0.0.0");
    }

    #[test]
    fn client_commands_quote_their_arguments() {
        assert_eq!(codex_add_command("/a b/launch", Some("/tmp/d.json")), "codex mcp add opsbatch-external --env OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE='/tmp/d.json' -- '/a b/launch'");
        assert_eq!(claude_add_command("/l", None), "claude mcp add -s user opsbatch-external -- '/l'");
        assert_eq!(grok_add_command("/l", Some("/d")), "grok mcp add opsbatch-external -e OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE='/d' -- '/l'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn config_snippets() {
        assert_eq!(toml_snippet("C:\\x\\l.exe", Some("/d")), "[mcp_servers.opsbatch-external]\ncommand = \"C:\\\\x\\\\l.exe\"\nargs = []\nenv = { OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE = \"/d\" }");
        let json: serde_json::Value = serde_json::from_str(&json_snippet("/l", Some("/d"))).unwrap();
        assert_eq!(json["mcpServers"]["opsbatch-external"]["command"], "/l");
        assert_eq!(json["mcpServers"]["opsbatch-external"]["env"]["OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE"], "/d");
        let none: serde_json::Value = serde_json::from_str(&json_snippet("/l", None)).unwrap();
        assert!(none["mcpServers"]["opsbatch-external"].get("env").is_none());
    }

    #[test]
    fn http_snippets_carry_the_token() {
        assert!(codex_http_toml("http://h/mcp").contains("bearer_token_env_var = \"OPSBATCH_MCP_BEARER_TOKEN\""));
        assert_eq!(codex_http_env_line("t'k"), "export OPSBATCH_MCP_BEARER_TOKEN='t'\\''k'");
        assert!(claude_http_add_command("http://h/mcp", "tok").contains("'Authorization: Bearer tok'"));
        let json: serde_json::Value = serde_json::from_str(&generic_http_json("http://h/mcp", "tok")).unwrap();
        assert_eq!(json["mcpServers"]["opsbatch-http"]["headers"]["Authorization"], "Bearer tok");
        assert_ne!(generate_token(), generate_token());
        assert_eq!(generate_token().len(), 64);
    }

    #[test]
    fn onboarding_prompt_depends_on_the_launcher() {
        assert!(onboarding_prompt(None, None).contains("turn on External MCP"));
        let p = onboarding_prompt(Some("/l"), Some("/d"));
        assert!(p.contains("- Command: /l") && p.contains("OPSBATCH_EXTERNAL_MCP_DISCOVERY_FILE=/d"));
    }
}
