//! Quick messages: reusable prompts that the chat composer offers as `/slug` commands (`quickMessages.ts`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct QuickMessage {
    pub id: String,
    /// Display label shown in pickers and settings.
    pub name: String,
    /// Slash-command token, e.g. `status` for `/status`.
    pub slug: String,
    /// Prompt text inserted into the composer when selected.
    pub content: String,
    pub description: Option<String>,
}

pub const NAME_LIMIT: usize = 120;
pub const SLUG_LIMIT: usize = 48;
pub const DESCRIPTION_LIMIT: usize = 240;
pub const CONTENT_LIMIT: usize = 10000;
pub const MAX_ITEMS: usize = 200;

/// Slugs the composer reserves for its own commands.
pub const SYSTEM_SLASH_COMMANDS: [&str; 2] = ["compact", "stop"];

/// Lower-cases, collapses every run of other characters to `-` and trims it.
pub fn normalize_slug(input: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in input.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    out.chars().take(SLUG_LIMIT).collect::<String>().trim_end_matches('-').to_string()
}

pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.split('-').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

pub fn slug_from_name(name: &str) -> String {
    normalize_slug(name)
}

pub fn create_id() -> String {
    format!("qm_{}_{}", chrono::Utc::now().timestamp_millis(), &uuid::Uuid::new_v4().simple().to_string()[..6])
}

fn clamp(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// `sanitizeQuickMessages`: drops incomplete rows and duplicate slugs, trims and clamps every field.
pub fn sanitize_quick_messages(raw: Vec<QuickMessage>) -> Vec<QuickMessage> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in raw {
        let id = clamp(entry.id.trim(), 64);
        let name = clamp(&entry.name, NAME_LIMIT).trim().to_string();
        let slug = normalize_slug(&clamp(&entry.slug, SLUG_LIMIT));
        let content = clamp(&entry.content, CONTENT_LIMIT).trim().to_string();
        let description = clamp(entry.description.as_deref().unwrap_or(""), DESCRIPTION_LIMIT).trim().to_string();
        if name.is_empty() || !is_valid_slug(&slug) || content.is_empty() || !seen.insert(slug.clone()) {
            continue;
        }
        out.push(QuickMessage { id: if id.is_empty() { create_id() } else { id }, name, slug, content, description: Some(description).filter(|d| !d.is_empty()) });
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    out
}

/// Slug-prefix or name-substring match, empty query keeps everything.
pub fn filter_quick_messages<'a>(messages: &'a [QuickMessage], query: &str) -> Vec<&'a QuickMessage> {
    let q = query.trim().to_lowercase();
    messages.iter().filter(|m| q.is_empty() || m.slug.to_lowercase().starts_with(&q) || m.name.to_lowercase().contains(&q)).collect()
}

/// The problem a draft slug has, if any (`QuickMessagesSettings` validation): empty, malformed, taken or reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlugIssue {
    Empty,
    Invalid,
    Duplicate,
    Reserved,
}

pub fn check_slug(slug: &str, editing_id: Option<&str>, messages: &[QuickMessage], reserved_skill_slugs: &[String]) -> Option<SlugIssue> {
    if slug.is_empty() {
        return Some(SlugIssue::Empty);
    }
    if !is_valid_slug(slug) {
        return Some(SlugIssue::Invalid);
    }
    if SYSTEM_SLASH_COMMANDS.contains(&slug) || reserved_skill_slugs.iter().any(|s| s == slug) {
        return Some(SlugIssue::Reserved);
    }
    if messages.iter().any(|m| m.slug == slug && Some(m.id.as_str()) != editing_id) {
        return Some(SlugIssue::Duplicate);
    }
    None
}

// ---------------------------------------------------------------------------
// Slash commands of the chat composer
// ---------------------------------------------------------------------------

/// What picking an entry of the `/` list does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashKind {
    /// Replaces the input with the message's prompt text.
    Quick(QuickMessage),
    /// Keeps `/slug ` in the input; the skill's instructions join the next request.
    Skill,
    /// `/search <query>`: web search results join the next request.
    Search,
    /// `/stop`: cancels the running answer.
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashItem {
    pub kind: SlashKind,
    pub slug: String,
    pub title: String,
    pub description: String,
}

/// The text after the leading `/` while the user is still typing the command (no whitespace yet).
pub fn slash_query(input: &str) -> Option<&str> {
    let rest = input.strip_prefix('/')?;
    (!rest.chars().any(char::is_whitespace)).then_some(rest)
}

/// `/slug rest of the message` -> `("slug", "rest of the message")`.
pub fn parse_slash_command(input: &str) -> Option<(&str, &str)> {
    let rest = input.trim_start().strip_prefix('/')?;
    let (slug, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    (!slug.is_empty()).then_some((slug, tail.trim()))
}

/// A user skill as the picker needs it: `(slug, name, description)`.
pub type SkillEntry = (String, String, String);

/// The entries of the `/` list for `query`: built-in commands first, then quick messages, then skills whose slug is
/// not taken by one of those.
pub fn build_slash_items(messages: &[QuickMessage], skills: &[SkillEntry], query: &str, search_ready: bool, streaming: bool) -> Vec<SlashItem> {
    let q = query.trim().to_lowercase();
    let matches = |slug: &str, name: &str| q.is_empty() || slug.to_lowercase().starts_with(&q) || name.to_lowercase().contains(&q);
    let mut items = Vec::new();
    if search_ready && matches("search", "Web search") {
        items.push(SlashItem { kind: SlashKind::Search, slug: "search".into(), title: "Web search".into(), description: String::new() });
    }
    if streaming && matches("stop", "Stop") {
        items.push(SlashItem { kind: SlashKind::Stop, slug: "stop".into(), title: "Stop".into(), description: String::new() });
    }
    let reserved: Vec<&str> = items.iter().map(|i| i.slug.as_str()).collect();
    let quick: Vec<SlashItem> = messages
        .iter()
        .filter(|m| !reserved.contains(&m.slug.as_str()) && matches(&m.slug, &m.name))
        .map(|m| SlashItem { kind: SlashKind::Quick(m.clone()), slug: m.slug.clone(), title: m.name.clone(), description: m.description.clone().unwrap_or_default() })
        .collect();
    let taken: Vec<String> = reserved.iter().map(|s| s.to_string()).chain(quick.iter().map(|i| i.slug.clone())).collect();
    items.extend(quick);
    items.extend(skills.iter().filter(|(slug, name, _)| !slug.is_empty() && !taken.contains(slug) && matches(slug, name)).map(|(slug, name, desc)| SlashItem { kind: SlashKind::Skill, slug: slug.clone(), title: name.clone(), description: desc.clone() }));
    items
}

/// Search results as the context block that goes in front of the question.
pub fn format_search_context(query: &str, results: &[crate::prefs::search::SearchResult]) -> String {
    if results.is_empty() {
        return format!("[网络搜索：{query}] 没有找到结果。");
    }
    let mut out = format!("[网络搜索结果：{query}]\n");
    for (i, r) in results.iter().enumerate() {
        let content: String = r.content.chars().take(600).collect();
        out.push_str(&format!("{}. {}\n{}\n{}\n", i + 1, r.title.trim(), r.url.trim(), content.trim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qm(name: &str, slug: &str, content: &str) -> QuickMessage {
        QuickMessage { id: String::new(), name: name.into(), slug: slug.into(), content: content.into(), description: None }
    }

    #[test]
    fn slugs_are_normalised() {
        assert_eq!(normalize_slug("  Check Disk Usage! "), "check-disk-usage");
        assert_eq!(normalize_slug("--a__b--"), "a-b");
        assert_eq!(normalize_slug("磁盘"), "");
        assert!(is_valid_slug("a-b-1"));
        assert!(!is_valid_slug("a--b"));
        assert!(!is_valid_slug("-a"));
        assert!(!is_valid_slug("A"));
        assert!(!is_valid_slug(""));
    }

    #[test]
    fn sanitize_drops_incomplete_and_duplicate_rows() {
        let list = sanitize_quick_messages(vec![qm(" Status ", "Status", " show status "), qm("x", "status", "dup"), qm("", "a", "b"), qm("n", "!!", "c"), qm("n", "ok", ""), qm("Two", "two", "2")]);
        assert_eq!(list.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(), vec!["status", "two"]);
        assert_eq!(list[0].name, "Status");
        assert_eq!(list[0].content, "show status");
        assert!(list[0].id.starts_with("qm_"));
    }

    #[test]
    fn sanitize_clamps_lengths_and_count() {
        let long = "x".repeat(CONTENT_LIMIT + 50);
        let list = sanitize_quick_messages(vec![qm("n", "a", &long)]);
        assert_eq!(list[0].content.chars().count(), CONTENT_LIMIT);
        let many: Vec<QuickMessage> = (0..MAX_ITEMS + 10).map(|i| qm("n", &format!("s{i}"), "c")).collect();
        assert_eq!(sanitize_quick_messages(many).len(), MAX_ITEMS);
    }

    #[test]
    fn filtering_matches_slug_prefix_or_name() {
        let list = sanitize_quick_messages(vec![qm("Disk usage", "disk", "c"), qm("Memory", "mem", "c")]);
        assert_eq!(filter_quick_messages(&list, "di").len(), 1);
        assert_eq!(filter_quick_messages(&list, "usage").len(), 1);
        assert_eq!(filter_quick_messages(&list, "isk").len(), 1);
        assert_eq!(filter_quick_messages(&list, "").len(), 2);
        assert_eq!(filter_quick_messages(&list, "zzz").len(), 0);
    }

    #[test]
    fn slash_input_is_recognised() {
        assert_eq!(slash_query("/di"), Some("di"));
        assert_eq!(slash_query("/"), Some(""));
        assert_eq!(slash_query("/search rust"), None);
        assert_eq!(slash_query("hello"), None);
        assert_eq!(parse_slash_command("/search rust gpui"), Some(("search", "rust gpui")));
        assert_eq!(parse_slash_command("  /deploy"), Some(("deploy", "")));
        assert_eq!(parse_slash_command("/"), None);
        assert_eq!(parse_slash_command("no slash"), None);
    }

    #[test]
    fn the_slash_list_orders_commands_messages_and_skills() {
        let messages = sanitize_quick_messages(vec![qm("Disk usage", "disk", "df -h"), qm("Deploy now", "deploy", "x")]);
        let skills = vec![("deploy".to_string(), "Deploy skill".to_string(), "d".to_string()), ("backup".to_string(), "Backup".to_string(), "b".to_string())];
        let slugs = |q: &str, search: bool, streaming: bool| build_slash_items(&messages, &skills, q, search, streaming).into_iter().map(|i| i.slug).collect::<Vec<_>>();
        assert_eq!(slugs("", true, true), vec!["search", "stop", "disk", "deploy", "backup"], "the skill named like a quick message is hidden");
        assert_eq!(slugs("", false, false), vec!["disk", "deploy", "backup"]);
        assert_eq!(slugs("d", false, false), vec!["disk", "deploy"]);
        assert_eq!(slugs("usage", false, false), vec!["disk"]);
        assert_eq!(slugs("se", true, false), vec!["search"]);
        assert_eq!(slugs("zzz", true, true), Vec::<String>::new());
        let items = build_slash_items(&messages, &skills, "disk", false, false);
        assert!(matches!(&items[0].kind, SlashKind::Quick(m) if m.content == "df -h"));
        assert!(matches!(build_slash_items(&messages, &skills, "back", false, false)[0].kind, SlashKind::Skill));
    }

    #[test]
    fn search_results_become_a_context_block() {
        use crate::prefs::search::SearchResult;
        let r = vec![SearchResult { title: " Rust ".into(), url: "https://rust-lang.org".into(), content: "A language".into() }];
        let text = format_search_context("rust", &r);
        assert!(text.starts_with("[网络搜索结果：rust]") && text.contains("1. Rust\nhttps://rust-lang.org\nA language"));
        assert!(format_search_context("x", &[]).contains("没有找到"));
    }

    #[test]
    fn slug_validation_reports_the_reason() {
        let list = sanitize_quick_messages(vec![qm("a", "taken", "c")]);
        let id = list[0].id.clone();
        let skills = vec!["deploy".to_string()];
        assert_eq!(check_slug("", None, &list, &skills), Some(SlugIssue::Empty));
        assert_eq!(check_slug("Bad Slug", None, &list, &skills), Some(SlugIssue::Invalid));
        assert_eq!(check_slug("stop", None, &list, &skills), Some(SlugIssue::Reserved));
        assert_eq!(check_slug("deploy", None, &list, &skills), Some(SlugIssue::Reserved));
        assert_eq!(check_slug("taken", None, &list, &skills), Some(SlugIssue::Duplicate));
        assert_eq!(check_slug("taken", Some(&id), &list, &skills), None);
        assert_eq!(check_slug("fresh", None, &list, &skills), None);
    }
}
