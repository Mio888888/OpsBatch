//! Add/edit-host form model: field defaults, the OS-switch rules, validation and conversion to the backend
//! `NewHost` / `UpdateHost`. Ports `handleHostOsChange` / `handleSaveHost` (MainLayout.tsx), `rdpSettings.ts` and
//! `proxySettings.ts`. Pure data — the dialog in `host_dialog.rs` only renders it.

use ops_core::security::SECRET_PLACEHOLDER;
use ops_ssh::hosts::{Host, NewHost, UpdateHost};
use serde_json::{json, Map, Value};

use crate::asset_tree::DEFAULT_GROUP_ID;

pub const DEFAULT_SSH_PORT: u32 = 22;
pub const DEFAULT_RDP_PORT: u32 = 3389;
pub const DEFAULT_VNC_PORT: u32 = 5900;
pub const MIN_RDP_DESKTOP_WIDTH: u32 = 640;
pub const MIN_RDP_DESKTOP_HEIGHT: u32 = 480;
pub const MAX_RDP_DESKTOP_WIDTH: u32 = 3840;
pub const MAX_RDP_DESKTOP_HEIGHT: u32 = 2160;
pub const DEFAULT_PROXY_PORT_HTTP: u32 = 8080;
pub const DEFAULT_PROXY_PORT_SOCKS5: u32 = 1080;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    Linux,
    Windows,
    Vnc,
}

impl OsKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OsKind::Linux => "linux",
            OsKind::Windows => "windows",
            OsKind::Vnc => "vnc",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "windows" => OsKind::Windows,
            "vnc" => OsKind::Vnc,
            _ => OsKind::Linux,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    Password,
    Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    Socks5,
    Http,
}

impl ProxyKind {
    pub fn default_port(self) -> u32 {
        match self {
            ProxyKind::Socks5 => DEFAULT_PROXY_PORT_SOCKS5,
            ProxyKind::Http => DEFAULT_PROXY_PORT_HTTP,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostFormValues {
    pub name: String,
    pub ip: String,
    pub port: String,
    pub auth: AuthKind,
    pub username: String,
    pub password: String,
    pub private_key: String,
    pub os: OsKind,
    pub group_id: String,
    pub remark: String,
    pub jump_chain: Vec<String>,

    pub rdp_domain: String,
    pub rdp_width: String,
    pub rdp_height: String,
    pub rdp_clipboard: bool,
    pub rdp_audio: bool,
    pub rdp_map_disk: bool,
    pub rdp_disk_path: String,

    pub vnc_port: String,
    pub vnc_username: String,
    pub vnc_password: String,
    pub vnc_view_only: bool,
    pub vnc_shared: bool,

    pub proxy_enabled: bool,
    pub proxy_type: ProxyKind,
    pub proxy_host: String,
    pub proxy_port: String,
    pub proxy_username: String,
    pub proxy_password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Name,
    Ip,
    Port,
    Username,
}

impl HostFormValues {
    /// Defaults of "add host" (`openNewHostModal`).
    pub fn new_default(group_id: Option<&str>) -> Self {
        Self {
            name: String::new(),
            ip: String::new(),
            port: DEFAULT_SSH_PORT.to_string(),
            auth: AuthKind::Password,
            username: "root".into(),
            password: String::new(),
            private_key: String::new(),
            os: OsKind::Linux,
            group_id: group_id.unwrap_or(DEFAULT_GROUP_ID).to_string(),
            remark: String::new(),
            jump_chain: Vec::new(),
            rdp_domain: String::new(),
            rdp_width: "1280".into(),
            rdp_height: "720".into(),
            rdp_clipboard: true,
            rdp_audio: true,
            rdp_map_disk: false,
            rdp_disk_path: String::new(),
            vnc_port: DEFAULT_VNC_PORT.to_string(),
            vnc_username: String::new(),
            vnc_password: String::new(),
            vnc_view_only: false,
            vnc_shared: true,
            proxy_enabled: false,
            proxy_type: ProxyKind::Socks5,
            proxy_host: String::new(),
            proxy_port: DEFAULT_PROXY_PORT_SOCKS5.to_string(),
            proxy_username: String::new(),
            proxy_password: String::new(),
        }
    }

    /// Prefill for "edit host" (`openEditHostModal`). Secrets arrive as the keychain placeholder; the password and
    /// key fields are left empty (= keep stored value), the VNC / proxy passwords keep the placeholder like before.
    pub fn from_host(host: &Host) -> Self {
        let rdp: Value = serde_json::from_str(&host.rdp_settings).unwrap_or(Value::Null);
        let proxy: Value = serde_json::from_str(&host.proxy_settings).unwrap_or(Value::Null);
        let rdp_str = |k: &str| rdp.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let rdp_num = |k: &str, d: u32| rdp.get(k).and_then(Value::as_f64).map(|v| (v as u32).to_string()).unwrap_or_else(|| d.to_string());
        let rdp_bool = |k: &str, d: bool| rdp.get(k).and_then(Value::as_bool).unwrap_or(d);
        let os = if host.os == "vnc" || rdp.get("protocol").and_then(Value::as_str) == Some("vnc") {
            OsKind::Vnc
        } else {
            OsKind::parse(&host.os)
        };
        let proxy_kind = match proxy.get("type").and_then(Value::as_str) {
            Some("http") => ProxyKind::Http,
            _ => ProxyKind::Socks5,
        };
        let proxy_on = proxy.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        Self {
            name: host.name.clone(),
            ip: host.ip.clone(),
            port: host.port.to_string(),
            auth: if host.auth_type == "key" { AuthKind::Key } else { AuthKind::Password },
            username: host.username.clone(),
            password: String::new(),
            private_key: String::new(),
            os,
            group_id: host.group_id.clone().unwrap_or_else(|| DEFAULT_GROUP_ID.into()),
            remark: host.remark.clone(),
            jump_chain: serde_json::from_str(&host.jump_chain).unwrap_or_default(),
            rdp_domain: rdp_str("domain"),
            rdp_width: rdp_num("desktopWidth", 1280),
            rdp_height: rdp_num("desktopHeight", 720),
            rdp_clipboard: rdp_bool("enableClipboard", true),
            rdp_audio: rdp_bool("enableAudio", true),
            rdp_map_disk: rdp_bool("mapDisk", false),
            rdp_disk_path: rdp_str("diskPath"),
            vnc_port: rdp_num("vncPort", DEFAULT_VNC_PORT),
            vnc_username: rdp_str("vncUsername"),
            vnc_password: rdp_str("vncPassword"),
            vnc_view_only: rdp_bool("vncViewOnly", false),
            vnc_shared: rdp_bool("vncShared", true),
            proxy_enabled: proxy_on,
            proxy_type: proxy_kind,
            proxy_host: proxy.get("host").and_then(Value::as_str).unwrap_or("").to_string(),
            proxy_port: proxy.get("port").and_then(Value::as_f64).map(|v| (v as u32).to_string()).unwrap_or_else(|| proxy_kind.default_port().to_string()),
            proxy_username: proxy.get("username").and_then(Value::as_str).unwrap_or("").to_string(),
            proxy_password: proxy.get("password").and_then(Value::as_str).unwrap_or("").to_string(),
        }
    }

    /// Switching the system type retargets port / username / auth and clears fields that no longer apply
    /// (`handleHostOsChange`). Returns true when the active tab must fall back to "basic".
    pub fn change_os(&mut self, next: OsKind) -> bool {
        let port = self.port.trim().parse::<u32>().ok();
        let user = self.username.trim().to_string();
        self.os = next;
        match next {
            OsKind::Windows => {
                if port.is_none() || port == Some(DEFAULT_SSH_PORT) || port == Some(DEFAULT_VNC_PORT) {
                    self.port = DEFAULT_RDP_PORT.to_string();
                }
                if user.is_empty() || user == "root" || user == "vnc" {
                    self.username = "Administrator".into();
                }
                self.auth = AuthKind::Password;
                self.private_key.clear();
                self.jump_chain.clear();
                if self.rdp_width.trim().is_empty() {
                    self.rdp_width = "1280".into();
                }
                if self.rdp_height.trim().is_empty() {
                    self.rdp_height = "720".into();
                }
                false
            }
            OsKind::Vnc => {
                if port.is_none() || port == Some(DEFAULT_SSH_PORT) || port == Some(DEFAULT_RDP_PORT) {
                    self.port = DEFAULT_VNC_PORT.to_string();
                }
                if user.is_empty() || user == "root" || user == "Administrator" {
                    self.username = "vnc".into();
                }
                self.auth = AuthKind::Password;
                self.password.clear();
                self.private_key.clear();
                self.jump_chain.clear();
                self.rdp_domain.clear();
                self.rdp_disk_path.clear();
                if self.vnc_port.trim().is_empty() {
                    self.vnc_port = DEFAULT_VNC_PORT.to_string();
                }
                true
            }
            OsKind::Linux => {
                if port.is_none() || port == Some(DEFAULT_RDP_PORT) || port == Some(DEFAULT_VNC_PORT) {
                    self.port = DEFAULT_SSH_PORT.to_string();
                }
                if user.is_empty() || user == "Administrator" || user == "vnc" {
                    self.username = "root".into();
                }
                self.rdp_domain.clear();
                self.rdp_disk_path.clear();
                true
            }
        }
    }

    pub fn change_proxy_type(&mut self, next: ProxyKind) {
        // Switching protocol resets a port that still holds the other protocol's default.
        let port = self.proxy_port.trim().parse::<u32>().ok();
        if port.is_none() || port == Some(self.proxy_type.default_port()) {
            self.proxy_port = next.default_port().to_string();
        }
        self.proxy_type = next;
    }

    /// Required-field errors (`rules: required`), in display order.
    pub fn validate(&self) -> Vec<Field> {
        let mut errors = Vec::new();
        if self.name.trim().is_empty() {
            errors.push(Field::Name);
        }
        if self.ip.trim().is_empty() {
            errors.push(Field::Ip);
        }
        if !matches!(self.port.trim().parse::<u32>(), Ok(1..=65535)) {
            errors.push(Field::Port);
        }
        if self.os != OsKind::Vnc && self.username.trim().is_empty() {
            errors.push(Field::Username);
        }
        errors
    }

    fn port_value(&self) -> i32 {
        self.port.trim().parse::<u32>().ok().filter(|p| (1..=65535).contains(p)).map(|p| p as i32).unwrap_or(match self.os {
            OsKind::Windows => DEFAULT_RDP_PORT as i32,
            OsKind::Vnc => DEFAULT_VNC_PORT as i32,
            OsKind::Linux => DEFAULT_SSH_PORT as i32,
        })
    }

    fn password_to_submit(&self) -> Option<String> {
        if self.os == OsKind::Vnc {
            None
        } else {
            submitted_secret(&self.password)
        }
    }

    fn private_key_to_submit(&self) -> Option<String> {
        if self.os == OsKind::Linux && self.auth == AuthKind::Key {
            submitted_secret(&self.private_key)
        } else if self.os == OsKind::Linux {
            // The original submits whatever is in the field regardless of the auth tab; it is empty unless typed.
            submitted_secret(&self.private_key)
        } else {
            None
        }
    }

    fn username_to_submit(&self) -> String {
        if self.os == OsKind::Vnc {
            let u = self.username.trim();
            if u.is_empty() { "vnc".into() } else { u.to_string() }
        } else {
            self.username.clone()
        }
    }

    fn auth_to_submit(&self) -> &'static str {
        if self.os != OsKind::Linux || self.auth == AuthKind::Password {
            "password"
        } else {
            "key"
        }
    }

    fn group_to_submit(&self) -> Option<String> {
        (!self.group_id.is_empty() && self.group_id != DEFAULT_GROUP_ID).then(|| self.group_id.clone())
    }

    fn jump_chain_json(&self) -> String {
        if self.os == OsKind::Linux {
            serde_json::to_string(&self.jump_chain).unwrap_or_else(|_| "[]".into())
        } else {
            "[]".into()
        }
    }

    /// `serializeRdpSettings(buildRdpSettings(values, os))`
    pub fn rdp_settings_json(&self) -> String {
        let mut m = Map::new();
        match self.os {
            OsKind::Vnc => {
                m.insert("protocol".into(), json!("vnc"));
                m.insert("vncPort".into(), json!(normalize_port(&self.vnc_port, DEFAULT_VNC_PORT)));
                if !self.vnc_username.trim().is_empty() {
                    m.insert("vncUsername".into(), json!(self.vnc_username.trim()));
                }
                if !self.vnc_password.trim().is_empty() {
                    m.insert("vncPassword".into(), json!(self.vnc_password.trim()));
                }
                // The client picks the type from the server's offer (a username means ARD); the stored method keeps the
                // Tauri build, which shares this database, doing the same.
                m.insert("vncAuthMethod".into(), json!(if self.vnc_username.trim().is_empty() { "vnc" } else { "ard" }));
                m.insert("vncViewOnly".into(), json!(self.vnc_view_only));
                m.insert("vncShared".into(), json!(self.vnc_shared));
            }
            OsKind::Windows => {
                m.insert("protocol".into(), json!("rdp"));
                if !self.rdp_domain.trim().is_empty() {
                    m.insert("domain".into(), json!(self.rdp_domain.trim()));
                }
                if let Some(w) = normalize_desktop(&self.rdp_width, MIN_RDP_DESKTOP_WIDTH, MAX_RDP_DESKTOP_WIDTH) {
                    m.insert("desktopWidth".into(), json!(w));
                }
                if let Some(h) = normalize_desktop(&self.rdp_height, MIN_RDP_DESKTOP_HEIGHT, MAX_RDP_DESKTOP_HEIGHT) {
                    m.insert("desktopHeight".into(), json!(h));
                }
                m.insert("enableClipboard".into(), json!(self.rdp_clipboard));
                m.insert("enableAudio".into(), json!(self.rdp_audio));
                m.insert("mapDisk".into(), json!(self.rdp_map_disk));
                if self.rdp_map_disk && !self.rdp_disk_path.trim().is_empty() {
                    m.insert("diskPath".into(), json!(self.rdp_disk_path.trim()));
                }
            }
            OsKind::Linux => {}
        }
        Value::Object(m).to_string()
    }

    /// `serializeProxySettings(buildProxySettings(values))`
    pub fn proxy_settings_json(&self) -> String {
        let host = self.proxy_host.trim();
        if !self.proxy_enabled || host.is_empty() {
            return "{}".into();
        }
        let mut m = Map::new();
        m.insert("enabled".into(), json!(true));
        m.insert("type".into(), json!(if self.proxy_type == ProxyKind::Http { "http" } else { "socks5" }));
        m.insert("host".into(), json!(host));
        m.insert("port".into(), json!(normalize_port(&self.proxy_port, self.proxy_type.default_port())));
        if !self.proxy_username.trim().is_empty() {
            m.insert("username".into(), json!(self.proxy_username.trim()));
        }
        if !self.proxy_password.trim().is_empty() {
            m.insert("password".into(), json!(self.proxy_password.trim()));
        }
        Value::Object(m).to_string()
    }

    pub fn to_new_host(&self) -> NewHost {
        NewHost {
            name: self.name.clone(),
            ip: self.ip.clone(),
            port: Some(self.port_value()),
            auth_type: Some(self.auth_to_submit().into()),
            username: Some(self.username_to_submit()),
            password: self.password_to_submit(),
            private_key: self.private_key_to_submit(),
            os: Some(self.os.as_str().into()),
            tags: Some("[]".into()),
            group_id: self.group_to_submit(),
            remark: Some(self.remark.clone()),
            jump_chain: Some(self.jump_chain_json()),
            rdp_settings: Some(self.rdp_settings_json()),
            proxy_settings: Some(self.proxy_settings_json()),
        }
    }

    /// `existing_tags` keeps the host's tags (the tag UI was removed in the Tauri build too).
    pub fn to_update_host(&self, id: &str, existing_tags: &str) -> UpdateHost {
        UpdateHost {
            id: id.to_string(),
            name: self.name.clone(),
            ip: self.ip.clone(),
            port: self.port_value(),
            auth_type: self.auth_to_submit().into(),
            username: self.username_to_submit(),
            password: self.password_to_submit(),
            private_key: self.private_key_to_submit(),
            os: self.os.as_str().into(),
            tags: existing_tags.to_string(),
            group_id: self.group_to_submit(),
            remark: Some(self.remark.clone()),
            jump_chain: Some(self.jump_chain_json()),
            rdp_settings: Some(self.rdp_settings_json()),
            proxy_settings: Some(self.proxy_settings_json()),
        }
    }
}

/// `submittedSecret`: blank or the keychain placeholder means "unchanged".
pub fn submitted_secret(value: &str) -> Option<String> {
    let t = value.trim();
    (!t.is_empty() && t != SECRET_PLACEHOLDER).then(|| value.to_string())
}

fn parse_number(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn normalize_desktop(s: &str, min: u32, max: u32) -> Option<u32> {
    parse_number(s).map(|v| (v.round() as i64).clamp(min as i64, max as i64) as u32)
}

fn normalize_port(s: &str, fallback: u32) -> u32 {
    parse_number(s).map(|v| (v.round() as i64).clamp(1, 65535) as u32).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_of(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn windows_form() -> HostFormValues {
        let mut f = HostFormValues::new_default(None);
        f.change_os(OsKind::Windows);
        f
    }

    // ---- ports of rdpSettingsDefaults.test.ts ----
    #[test]
    fn remote_audio_is_enabled_by_default_for_windows() {
        let f = windows_form();
        assert_eq!(json_of(&f.rdp_settings_json())["enableAudio"], json!(true));
        assert!(HostFormValues::new_default(None).rdp_audio, "form default");
    }

    #[test]
    fn explicit_audio_opt_out_is_kept() {
        let mut f = windows_form();
        f.rdp_audio = false;
        assert_eq!(json_of(&f.rdp_settings_json())["enableAudio"], json!(false));
    }

    #[test]
    fn rdp_desktop_size_is_clamped_and_disk_path_needs_map_disk() {
        let mut f = windows_form();
        f.rdp_width = "100".into();
        f.rdp_height = "99999".into();
        f.rdp_disk_path = " /tmp/share ".into();
        let v = json_of(&f.rdp_settings_json());
        assert_eq!((v["desktopWidth"].clone(), v["desktopHeight"].clone()), (json!(640), json!(2160)));
        assert!(v.get("diskPath").is_none(), "ignored while mapDisk is off");
        f.rdp_map_disk = true;
        assert_eq!(json_of(&f.rdp_settings_json())["diskPath"], json!("/tmp/share"));
        f.rdp_width = "abc".into();
        assert!(json_of(&f.rdp_settings_json()).get("desktopWidth").is_none());
    }

    #[test]
    fn vnc_settings_json() {
        let mut f = HostFormValues::new_default(None);
        f.change_os(OsKind::Vnc);
        f.vnc_username = " me ".into();
        f.vnc_password = " pw ".into();
        let v = json_of(&f.rdp_settings_json());
        assert_eq!(v["protocol"], json!("vnc"));
        assert_eq!(v["vncPort"], json!(5900));
        assert_eq!((v["vncUsername"].clone(), v["vncPassword"].clone()), (json!("me"), json!("pw")));
        assert_eq!((v["vncAuthMethod"].clone(), v["vncViewOnly"].clone(), v["vncShared"].clone()), (json!("ard"), json!(false), json!(true)));
        f.vnc_port = "99999".into();
        assert_eq!(json_of(&f.rdp_settings_json())["vncPort"], json!(65535));
        // no username: password-only VNC auth
        f.vnc_username = "  ".into();
        let v = json_of(&f.rdp_settings_json());
        assert_eq!((v.get("vncUsername"), v["vncAuthMethod"].clone()), (None, json!("vnc")));
    }

    #[test]
    fn linux_hosts_store_empty_rdp_settings() {
        assert_eq!(HostFormValues::new_default(None).rdp_settings_json(), "{}");
    }

    // ---- ports of proxySettings.test.ts ----
    #[test]
    fn proxy_settings_are_normalised_from_form_values() {
        let mut f = HostFormValues::new_default(None);
        f.proxy_enabled = true;
        f.proxy_type = ProxyKind::Socks5;
        f.proxy_host = " proxy.internal ".into();
        f.proxy_port = "1080".into();
        f.proxy_username = " deploy ".into();
        f.proxy_password = " secret ".into();
        assert_eq!(
            json_of(&f.proxy_settings_json()),
            json!({"enabled": true, "type": "socks5", "host": "proxy.internal", "port": 1080, "username": "deploy", "password": "secret"})
        );
    }

    #[test]
    fn incomplete_or_disabled_proxy_is_dropped() {
        let mut f = HostFormValues::new_default(None);
        f.proxy_enabled = true;
        f.proxy_type = ProxyKind::Http;
        f.proxy_host = "".into();
        assert_eq!(f.proxy_settings_json(), "{}");
        f.proxy_enabled = false;
        f.proxy_host = "proxy".into();
        assert_eq!(f.proxy_settings_json(), "{}");
    }

    #[test]
    fn proxy_round_trips_through_the_host_row() {
        let mut f = HostFormValues::new_default(None);
        f.name = "n".into();
        f.ip = "1.2.3.4".into();
        f.proxy_enabled = true;
        f.change_proxy_type(ProxyKind::Http);
        f.proxy_host = "proxy.internal".into();
        assert_eq!(f.proxy_port, "8080", "http default");
        let nh = f.to_new_host();
        let host = host_from_new(&nh);
        let back = HostFormValues::from_host(&host);
        assert!(back.proxy_enabled);
        assert_eq!((back.proxy_type, back.proxy_host.as_str(), back.proxy_port.as_str()), (ProxyKind::Http, "proxy.internal", "8080"));
    }

    fn host_from_new(n: &NewHost) -> Host {
        Host {
            id: "id".into(),
            name: n.name.clone(),
            ip: n.ip.clone(),
            port: n.port.unwrap_or(22),
            auth_type: n.auth_type.clone().unwrap_or_default(),
            username: n.username.clone().unwrap_or_default(),
            password: n.password.clone(),
            private_key: n.private_key.clone(),
            os: n.os.clone().unwrap_or_default(),
            tags: n.tags.clone().unwrap_or_default(),
            group_id: n.group_id.clone(),
            remark: n.remark.clone().unwrap_or_default(),
            status: "unknown".into(),
            jump_chain: n.jump_chain.clone().unwrap_or_default(),
            rdp_settings: n.rdp_settings.clone().unwrap_or_default(),
            proxy_settings: n.proxy_settings.clone().unwrap_or_default(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    // ---- OS switching ----
    #[test]
    fn switching_os_retargets_port_username_and_clears_inapplicable_fields() {
        let mut f = HostFormValues::new_default(None);
        f.private_key = "KEY".into();
        f.jump_chain = vec!["h1".into()];
        assert!(!f.change_os(OsKind::Windows));
        assert_eq!((f.port.as_str(), f.username.as_str(), f.auth), ("3389", "Administrator", AuthKind::Password));
        assert!(f.private_key.is_empty() && f.jump_chain.is_empty());

        assert!(f.change_os(OsKind::Vnc), "VNC returns to the basic tab");
        assert_eq!((f.port.as_str(), f.username.as_str()), ("5900", "vnc"));

        f.change_os(OsKind::Linux);
        assert_eq!((f.port.as_str(), f.username.as_str()), ("22", "root"));

        // custom values survive a switch
        let mut g = HostFormValues::new_default(None);
        g.port = "2222".into();
        g.username = "deploy".into();
        g.change_os(OsKind::Windows);
        assert_eq!((g.port.as_str(), g.username.as_str()), ("2222", "deploy"));
        g.change_os(OsKind::Linux);
        assert_eq!((g.port.as_str(), g.username.as_str()), ("2222", "deploy"));
    }

    // ---- validation + submission ----
    #[test]
    fn required_fields_and_port_range() {
        let mut f = HostFormValues::new_default(None);
        assert_eq!(f.validate(), vec![Field::Name, Field::Ip]);
        f.name = "a".into();
        f.ip = "b".into();
        assert!(f.validate().is_empty());
        f.port = "0".into();
        assert_eq!(f.validate(), vec![Field::Port]);
        f.port = "70000".into();
        assert_eq!(f.validate(), vec![Field::Port]);
        f.port = "22".into();
        f.username = "  ".into();
        assert_eq!(f.validate(), vec![Field::Username]);
        f.change_os(OsKind::Vnc);
        assert!(f.validate().is_empty(), "VNC has no ssh user");
    }

    #[test]
    fn secrets_blank_or_placeholder_mean_keep() {
        assert_eq!(submitted_secret(""), None);
        assert_eq!(submitted_secret("  "), None);
        assert_eq!(submitted_secret(SECRET_PLACEHOLDER), None);
        assert_eq!(submitted_secret("hunter2").as_deref(), Some("hunter2"));
    }

    #[test]
    fn submission_shapes_per_os() {
        let mut f = HostFormValues::new_default(Some("g1"));
        f.name = "web".into();
        f.ip = "10.0.0.1".into();
        f.password = "pw".into();
        f.jump_chain = vec!["j1".into()];
        let n = f.to_new_host();
        assert_eq!((n.auth_type.as_deref(), n.password.as_deref(), n.group_id.as_deref(), n.os.as_deref()), (Some("password"), Some("pw"), Some("g1"), Some("linux")));
        assert_eq!(n.jump_chain.as_deref(), Some("[\"j1\"]"));

        f.auth = AuthKind::Key;
        f.private_key = "-----KEY-----".into();
        f.group_id = DEFAULT_GROUP_ID.into();
        let n = f.to_new_host();
        assert_eq!((n.auth_type.as_deref(), n.private_key.as_deref(), n.group_id), (Some("key"), Some("-----KEY-----"), None));

        f.change_os(OsKind::Windows);
        f.password = "winpw".into();
        let n = f.to_new_host();
        assert_eq!((n.auth_type.as_deref(), n.private_key, n.jump_chain.as_deref()), (Some("password"), None, Some("[]")));

        f.change_os(OsKind::Vnc);
        f.username = "".into();
        f.password = "ignored".into();
        let n = f.to_new_host();
        assert_eq!((n.username.as_deref(), n.password, n.port), (Some("vnc"), None, Some(5900)));
    }

    #[test]
    fn editing_prefills_from_the_row_and_keeps_secrets_on_blank() {
        let mut f = HostFormValues::new_default(None);
        f.name = "db".into();
        f.ip = "10.1.1.1".into();
        f.password = "pw".into();
        f.remark = "主库".into();
        let mut host = host_from_new(&f.to_new_host());
        host.id = "h9".into();
        host.password = Some(SECRET_PLACEHOLDER.into());
        host.tags = "[\"x\"]".into();
        let edit = HostFormValues::from_host(&host);
        assert_eq!((edit.name.as_str(), edit.remark.as_str(), edit.password.as_str()), ("db", "主库", ""));
        let upd = edit.to_update_host("h9", &host.tags);
        assert_eq!((upd.id.as_str(), upd.password, upd.tags.as_str()), ("h9", None, "[\"x\"]"));
    }

    #[test]
    fn vnc_rows_open_as_vnc_even_when_os_says_linux() {
        let mut f = HostFormValues::new_default(None);
        f.name = "v".into();
        f.ip = "1.1.1.1".into();
        f.change_os(OsKind::Vnc);
        let mut host = host_from_new(&f.to_new_host());
        host.os = "linux".into();
        assert_eq!(HostFormValues::from_host(&host).os, OsKind::Vnc);
    }
}
