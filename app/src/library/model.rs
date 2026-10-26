//! Library entries as the UI sees them (parsed tags/parameters) plus the pure filtering/formatting helpers
//! shared by the command, script and quick-action pages and the inline panels.

use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use ops_library::library::{CustomCommand, CustomScript, NewCommand, NewScript};
use ops_library::quick_actions::{NewQuickAction, QuickAction};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Param {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "defaultValue")]
    pub default_value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Risk {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

impl Risk {
    pub const ALL: [Risk; 4] = [Risk::Low, Risk::Medium, Risk::High, Risk::Critical];

    pub fn parse(s: &str) -> Self {
        match s {
            "medium" => Risk::Medium,
            "high" => Risk::High,
            "critical" => Risk::Critical,
            _ => Risk::Low,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
            Risk::Critical => "critical",
        }
    }
    pub fn label_key(self) -> &'static str {
        match self {
            Risk::Low => "library.risk.low",
            Risk::Medium => "library.risk.medium",
            Risk::High => "library.risk.high",
            Risk::Critical => "library.risk.critical",
        }
    }
    pub fn option_key(self) -> &'static str {
        match self {
            Risk::Low => "library.risk.lowOption",
            Risk::Medium => "library.risk.mediumOption",
            Risk::High => "library.risk.highOption",
            Risk::Critical => "library.risk.criticalOption",
        }
    }
    pub fn is_high(self) -> bool {
        matches!(self, Risk::High | Risk::Critical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Platform {
    #[default]
    Linux,
    Windows,
    Both,
}

impl Platform {
    pub fn parse(s: &str) -> Self {
        match s {
            "windows" => Platform::Windows,
            "both" => Platform::Both,
            _ => Platform::Linux,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Windows => "windows",
            Platform::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Command,
    Docker,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandEntry {
    pub id: String,
    pub name: String,
    pub command: String,
    pub url: String,
    pub kind: Kind,
    pub category: String,
    pub tags: Vec<String>,
    pub risk: Risk,
    pub description: String,
    pub platform: Platform,
    pub parameters: Vec<Param>,
    pub starred: bool,
    pub is_builtin: bool,
}

fn parse_tags(raw: &str) -> Vec<String> {
    serde_json::from_str(if raw.is_empty() { "[]" } else { raw }).unwrap_or_default()
}

fn parse_params(raw: &str) -> Vec<Param> {
    serde_json::from_str(if raw.is_empty() { "[]" } else { raw }).unwrap_or_default()
}

impl CommandEntry {
    pub fn from_row(c: &CustomCommand) -> Self {
        let tags = parse_tags(&c.tags);
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            command: c.command.clone(),
            url: c.url.clone(),
            kind: if tags.iter().any(|t| t == "docker") { Kind::Docker } else { Kind::Command },
            category: c.category.clone(),
            tags,
            risk: Risk::parse(&c.risk),
            description: c.description.clone(),
            platform: Platform::parse(&c.platform),
            parameters: parse_params(&c.parameters),
            starred: c.starred,
            is_builtin: c.is_builtin,
        }
    }

    /// Docker commands carry a `docker` tag; ordinary ones never do.
    fn stored_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self.tags.iter().filter(|t| *t != "docker").cloned().collect();
        if self.kind == Kind::Docker {
            tags.push("docker".into());
        }
        tags
    }

    pub fn to_new(&self) -> NewCommand {
        NewCommand {
            name: self.name.clone(),
            command: self.command.clone(),
            category: Some(self.category.clone()),
            tags: Some(serde_json::to_string(&self.stored_tags()).unwrap()),
            risk: Some(self.risk.as_str().into()),
            description: Some(self.description.clone()),
            platform: Some(self.platform.as_str().into()),
            parameters: Some(serde_json::to_string(&self.parameters).unwrap()),
            url: Some(self.url.clone()),
        }
    }

    /// The text that is copied/inserted: remote script URLs become a `curl | bash` line.
    pub fn display_command(&self) -> String {
        if self.url.is_empty() { self.command.clone() } else { format!("curl -sSL '{}' | bash", self.url) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptEntry {
    pub id: String,
    pub name: String,
    pub language: String,
    pub category: String,
    pub tags: Vec<String>,
    pub risk: Risk,
    pub description: String,
    pub content: String,
    pub parameters: Vec<Param>,
    pub url: String,
    pub platform: Platform,
    pub starred: bool,
    pub is_builtin: bool,
}

impl ScriptEntry {
    pub fn from_row(s: &CustomScript) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            language: s.language.clone(),
            category: s.category.clone(),
            tags: parse_tags(&s.tags),
            risk: Risk::parse(&s.risk),
            description: s.description.clone(),
            content: s.content.clone(),
            parameters: parse_params(&s.parameters),
            url: s.url.clone(),
            platform: Platform::parse(&s.platform),
            starred: s.starred,
            is_builtin: s.is_builtin,
        }
    }

    pub fn to_new(&self) -> NewScript {
        NewScript {
            name: self.name.clone(),
            language: Some(self.language.clone()),
            category: Some(self.category.clone()),
            tags: Some(serde_json::to_string(&self.tags).unwrap()),
            risk: Some(self.risk.as_str().into()),
            description: Some(self.description.clone()),
            content: Some(self.content.clone()),
            parameters: Some(serde_json::to_string(&self.parameters).unwrap()),
            url: Some(self.url.clone()),
            platform: Some(self.platform.as_str().into()),
        }
    }

    pub fn to_row(&self) -> CustomScript {
        CustomScript {
            id: self.id.clone(),
            name: self.name.clone(),
            language: self.language.clone(),
            category: self.category.clone(),
            tags: serde_json::to_string(&self.tags).unwrap(),
            risk: self.risk.as_str().into(),
            description: self.description.clone(),
            content: self.content.clone(),
            parameters: serde_json::to_string(&self.parameters).unwrap(),
            url: self.url.clone(),
            platform: self.platform.as_str().into(),
            starred: self.starred,
            is_builtin: self.is_builtin,
        }
    }

    pub fn display_text(&self) -> String {
        if self.url.is_empty() { self.content.clone() } else { format!("curl -sSL '{}' | bash", self.url) }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuickActionEntry {
    pub id: String,
    pub name: String,
    pub command: String,
    pub category: String,
    pub parameters_json: String,
    pub order: i32,
    pub starred: bool,
    pub description: String,
    pub tags: Vec<String>,
    pub language: String,
    pub last_run_at: String,
    pub last_status: String,
}

impl QuickActionEntry {
    pub fn from_row(a: &QuickAction) -> Self {
        Self {
            id: a.id.clone(),
            name: a.name.clone(),
            command: a.command.clone(),
            category: a.category.clone(),
            parameters_json: if a.parameters.is_empty() { "[]".into() } else { a.parameters.clone() },
            order: a.sort_order,
            starred: a.starred == 1,
            description: a.description.clone(),
            tags: parse_tags(&a.tags),
            language: if a.language.is_empty() { "shell".into() } else { a.language.clone() },
            last_run_at: a.last_run_at.clone(),
            last_status: a.last_status.clone(),
        }
    }

    pub fn to_row(&self) -> QuickAction {
        QuickAction {
            id: self.id.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            category: self.category.clone(),
            parameters: self.parameters_json.clone(),
            sort_order: self.order,
            starred: self.starred as i32,
            description: self.description.clone(),
            tags: serde_json::to_string(&self.tags).unwrap(),
            language: self.language.clone(),
            last_run_at: self.last_run_at.clone(),
            last_status: self.last_status.clone(),
        }
    }

    pub fn to_new(&self) -> NewQuickAction {
        NewQuickAction {
            name: self.name.clone(),
            command: self.command.clone(),
            category: Some(self.category.clone()),
            parameters: Some(self.parameters_json.clone()),
            sort_order: None,
            description: Some(self.description.clone()),
            tags: Some(serde_json::to_string(&self.tags).unwrap()),
            language: Some(self.language.clone()),
        }
    }
}

// ---- filtering ----

/// Category -> number of entries (blank categories are not listed).
pub fn category_counts<'a>(categories: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for c in categories {
        let c = c.trim();
        if !c.is_empty() {
            *m.entry(c.to_string()).or_insert(0) += 1;
        }
    }
    m
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(needle)
}

/// Commands matching the docker/command split, category and query (name, command, description, tags).
pub fn filter_commands<'a>(all: &'a [CommandEntry], docker_only: bool, category: Option<&str>, query: &str) -> Vec<&'a CommandEntry> {
    let q = query.trim().to_lowercase();
    all.iter()
        .filter(|c| docker_only == (c.kind == Kind::Docker))
        .filter(|c| category.map_or(true, |cat| c.category == cat))
        .filter(|c| q.is_empty() || contains_ci(&c.name, &q) || contains_ci(&c.command, &q) || contains_ci(&c.description, &q) || c.tags.iter().any(|t| contains_ci(t, &q)))
        .collect()
}

/// Scripts by category and query (name, description, tags).
pub fn filter_scripts<'a>(all: &'a [ScriptEntry], category: Option<&str>, query: &str) -> Vec<&'a ScriptEntry> {
    let q = query.trim().to_lowercase();
    all.iter()
        .filter(|s| category.map_or(true, |cat| s.category == cat))
        .filter(|s| q.is_empty() || contains_ci(&s.name, &q) || contains_ci(&s.description, &q) || s.tags.iter().any(|t| contains_ci(t, &q)))
        .collect()
}

/// Quick actions by category and query (name, command, description).
pub fn filter_actions<'a>(all: &'a [QuickActionEntry], category: Option<&str>, query: &str) -> Vec<&'a QuickActionEntry> {
    let q = query.trim().to_lowercase();
    all.iter()
        .filter(|a| category.map_or(true, |cat| a.category == cat))
        .filter(|a| q.is_empty() || contains_ci(&a.name, &q) || contains_ci(&a.command, &q) || contains_ci(&a.description, &q))
        .collect()
}

/// Categories offered when creating something: the presets first, then any extra existing ones.
pub fn category_options(presets: &[&str], existing: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = presets.iter().map(|s| s.to_string()).collect();
    for c in existing {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// "Just now / N minutes ago / ..." bucket for a `last_run_at` timestamp (`YYYY-MM-DD HH:MM:SS`, local time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastRun {
    JustNow,
    Minutes(i64),
    Hours(i64),
    Days(i64),
    Date(String),
    Unknown,
}

pub fn last_run_bucket(ts: &str, now: NaiveDateTime) -> LastRun {
    if ts.is_empty() {
        return LastRun::Unknown;
    }
    let Ok(then) = NaiveDateTime::parse_from_str(&ts.replace('T', " "), "%Y-%m-%d %H:%M:%S") else { return LastRun::Unknown };
    let min = (now - then).num_minutes();
    if min < 1 {
        LastRun::JustNow
    } else if min < 60 {
        LastRun::Minutes(min)
    } else if min / 60 < 24 {
        LastRun::Hours(min / 60)
    } else if min / 60 / 24 < 30 {
        LastRun::Days(min / 60 / 24)
    } else {
        LastRun::Date(then.format("%Y-%m-%d").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, cmd: &str, tags: &str, cat: &str) -> CustomCommand {
        CustomCommand { id: name.into(), name: name.into(), command: cmd.into(), category: cat.into(), tags: tags.into(), risk: "high".into(), description: format!("about {name}"), platform: "both".into(), parameters: r#"[{"name":"n","description":"d","required":true,"defaultValue":"5"}]"#.into(), url: String::new(), starred: false, is_builtin: false }
    }

    #[test]
    fn command_rows_parse_tags_params_and_kind() {
        let e = CommandEntry::from_row(&row("ps", "docker ps", r#"["docker","containers"]"#, "docker"));
        assert_eq!(e.kind, Kind::Docker);
        assert_eq!(e.risk, Risk::High);
        assert!(e.risk.is_high());
        assert_eq!(e.platform, Platform::Both);
        assert_eq!(e.parameters, vec![Param { name: "n".into(), description: "d".into(), required: true, default_value: "5".into() }]);
        // Garbage JSON degrades to empty rather than failing the whole list.
        let bad = CommandEntry::from_row(&row("x", "x", "not json", ""));
        assert!(bad.tags.is_empty());
    }

    #[test]
    fn docker_tag_round_trips_through_storage() {
        let mut e = CommandEntry::from_row(&row("ls", "ls", r#"["a"]"#, "sys"));
        assert_eq!(e.to_new().tags.as_deref(), Some(r#"["a"]"#));
        e.kind = Kind::Docker;
        assert_eq!(e.to_new().tags.as_deref(), Some(r#"["a","docker"]"#));
        e.tags = vec!["a".into(), "docker".into()];
        e.kind = Kind::Command;
        assert_eq!(e.to_new().tags.as_deref(), Some(r#"["a"]"#), "docker tag removed for plain commands");
    }

    #[test]
    fn remote_commands_display_as_curl_pipe_bash() {
        let mut e = CommandEntry::from_row(&row("i", "ignored", "[]", "x"));
        assert_eq!(e.display_command(), "ignored");
        e.url = "https://x/y.sh".into();
        assert_eq!(e.display_command(), "curl -sSL 'https://x/y.sh' | bash");
    }

    #[test]
    fn filters_split_docker_and_match_text_case_insensitively() {
        let all = vec![
            CommandEntry::from_row(&row("Disk usage", "df -h", "[]", "disk")),
            CommandEntry::from_row(&row("Docker ps", "docker ps", r#"["docker"]"#, "docker")),
            CommandEntry::from_row(&row("Memory", "free -m", r#"["ram"]"#, "memory")),
        ];
        assert_eq!(filter_commands(&all, false, None, "").len(), 2);
        assert_eq!(filter_commands(&all, true, None, "").len(), 1);
        assert_eq!(filter_commands(&all, false, Some("disk"), "").len(), 1);
        assert_eq!(filter_commands(&all, false, None, "DF -H").len(), 1, "matches the command text");
        assert_eq!(filter_commands(&all, false, None, "RAM").len(), 1, "matches tags");
        assert_eq!(filter_commands(&all, false, None, "about memory").len(), 1, "matches the description");
        assert!(filter_commands(&all, false, None, "nope").is_empty());
    }

    #[test]
    fn category_counts_skip_blanks_and_sort() {
        let m = category_counts(["net", "disk", " ", "net", ""]);
        assert_eq!(m.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(), [("disk", 1), ("net", 2)]);
    }

    #[test]
    fn category_options_put_presets_first_without_duplicates() {
        let o = category_options(&["a", "b"], ["b".to_string(), "c".to_string()]);
        assert_eq!(o, ["a", "b", "c"]);
    }

    #[test]
    fn last_run_buckets() {
        let now = NaiveDateTime::parse_from_str("2026-10-23 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(last_run_bucket("2026-10-23 11:59:30", now), LastRun::JustNow);
        assert_eq!(last_run_bucket("2026-10-23 11:30:00", now), LastRun::Minutes(30));
        assert_eq!(last_run_bucket("2026-10-23 07:00:00", now), LastRun::Hours(5));
        assert_eq!(last_run_bucket("2026-10-20 12:00:00", now), LastRun::Days(3));
        assert_eq!(last_run_bucket("2026-08-01 12:00:00", now), LastRun::Date("2026-08-01".into()));
        assert_eq!(last_run_bucket("", now), LastRun::Unknown);
        assert_eq!(last_run_bucket("garbage", now), LastRun::Unknown);
    }

    #[test]
    fn quick_action_round_trip() {
        let row = QuickAction { id: "1".into(), name: "n".into(), command: "echo".into(), category: "c".into(), parameters: String::new(), sort_order: 2, starred: 1, description: String::new(), tags: r#"["t"]"#.into(), language: String::new(), last_run_at: String::new(), last_status: "success".into() };
        let e = QuickActionEntry::from_row(&row);
        assert_eq!((e.language.as_str(), e.parameters_json.as_str(), e.starred), ("shell", "[]", true));
        let back = e.to_row();
        assert_eq!((back.starred, back.sort_order, back.tags.as_str()), (1, 2, r#"["t"]"#));
    }
}
