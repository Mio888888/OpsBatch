#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCloseMode {
    All,
    Others,
    Left,
    Right,
}

/// Which tabs to close for the tab context menu entries (close all / others / left / right).
pub fn terminal_tab_close_targets(
    tab_keys: &[String],
    target_key: &str,
    mode: TabCloseMode,
) -> Vec<String> {
    if mode == TabCloseMode::All {
        return tab_keys.to_vec();
    }
    let Some(idx) = tab_keys.iter().position(|k| k == target_key) else {
        return Vec::new();
    };
    match mode {
        TabCloseMode::Others => tab_keys
            .iter()
            .filter(|k| *k != target_key)
            .cloned()
            .collect(),
        TabCloseMode::Left => tab_keys[..idx].to_vec(),
        TabCloseMode::Right => tab_keys[idx + 1..].to_vec(),
        TabCloseMode::All => unreachable!(),
    }
}

/// Which tab becomes active after closing `closing`. Keeps the current one when it survives; otherwise the preferred
/// tab (if still open), then the tab that took the closed one's place, then the one before it.
pub fn active_key_after_close(
    tab_keys: &[String],
    closing: &[String],
    active: Option<&str>,
    preferred: Option<&str>,
) -> Option<String> {
    let remaining: Vec<&String> = tab_keys.iter().filter(|k| !closing.contains(k)).collect();
    if let Some(a) = active.filter(|a| remaining.iter().any(|k| k.as_str() == *a)) {
        return Some(a.to_string());
    }
    if let Some(p) = preferred.filter(|p| remaining.iter().any(|k| k.as_str() == *p)) {
        return Some(p.to_string());
    }
    let first_closing = tab_keys
        .iter()
        .position(|k| closing.contains(k))
        .unwrap_or(0);
    remaining
        .get(first_closing)
        .or_else(|| first_closing.checked_sub(1).and_then(|i| remaining.get(i)))
        .or_else(|| remaining.last())
        .map(|k| (*k).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        ["local", "web-01", "db-01", "cache-01"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn close_all_selects_everything() {
        assert_eq!(
            terminal_tab_close_targets(&keys(), "web-01", TabCloseMode::All),
            keys()
        );
    }
    #[test]
    fn close_others() {
        assert_eq!(
            terminal_tab_close_targets(&keys(), "web-01", TabCloseMode::Others),
            v(&["local", "db-01", "cache-01"])
        );
    }
    #[test]
    fn close_left() {
        assert_eq!(
            terminal_tab_close_targets(&keys(), "db-01", TabCloseMode::Left),
            v(&["local", "web-01"])
        );
    }
    #[test]
    fn close_right() {
        assert_eq!(
            terminal_tab_close_targets(&keys(), "db-01", TabCloseMode::Right),
            v(&["cache-01"])
        );
    }
    #[test]
    fn missing_target_closes_nothing_except_close_all() {
        assert!(terminal_tab_close_targets(&keys(), "missing", TabCloseMode::Left).is_empty());
        assert_eq!(
            terminal_tab_close_targets(&keys(), "missing", TabCloseMode::All).len(),
            4
        );
    }

    #[test]
    fn active_tab_after_closing() {
        let keys = keys();
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // closing a background tab keeps the active one
        assert_eq!(
            active_key_after_close(&keys, &c(&["db-01"]), Some("web-01"), None).as_deref(),
            Some("web-01")
        );
        // closing the active tab selects the neighbour that takes its place
        assert_eq!(
            active_key_after_close(&keys, &c(&["db-01"]), Some("db-01"), None).as_deref(),
            Some("cache-01")
        );
        // ... or the previous one when it was last
        assert_eq!(
            active_key_after_close(&keys, &c(&["cache-01"]), Some("cache-01"), None).as_deref(),
            Some("db-01")
        );
        // close others keeps the clicked tab
        assert_eq!(
            active_key_after_close(
                &keys,
                &c(&["local", "db-01", "cache-01"]),
                Some("db-01"),
                Some("web-01")
            )
            .as_deref(),
            Some("web-01")
        );
        // close all
        assert_eq!(
            active_key_after_close(&keys, &keys, Some("local"), None),
            None
        );
        // close left of the clicked tab
        assert_eq!(
            active_key_after_close(
                &keys,
                &c(&["local", "web-01"]),
                Some("local"),
                Some("db-01")
            )
            .as_deref(),
            Some("db-01")
        );
    }
}
