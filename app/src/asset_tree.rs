//! Pure model of the host tree in the asset sidebar (port of the `useMemo` blocks in MainLayout.tsx):
//! filtering, grouping, auto-expansion and tri-state selection. No UI types in here so it is unit-testable.

use std::collections::{BTreeSet, HashMap, HashSet};

use ops_ssh::groups::HostGroup;
use ops_ssh::hosts::Host;

pub const DEFAULT_GROUP_ID: &str = "__default__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    Linux,
    Windows,
    Vnc,
}

impl HostKind {
    pub fn of(host: &Host) -> Self {
        if host.os == "vnc" || rdp_protocol_is_vnc(&host.rdp_settings) {
            HostKind::Vnc
        } else if host.os == "windows" {
            HostKind::Windows
        } else {
            HostKind::Linux
        }
    }
}

fn rdp_protocol_is_vnc(rdp_settings: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(rdp_settings)
        .ok()
        .and_then(|v| v.get("protocol").and_then(|p| p.as_str().map(str::to_owned)))
        .as_deref()
        == Some("vnc")
}

/// Case-insensitive match on name / ip / username / remark (same fields as the Tauri sidebar).
pub fn host_matches(host: &Host, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    [host.name.as_str(), host.ip.as_str(), host.username.as_str(), host.remark.as_str()]
        .join(" ")
        .to_lowercase()
        .contains(&q)
}

/// Expansion state: groups holding hosts open automatically; the user can still collapse/expand any group.
#[derive(Debug, Default, Clone)]
pub struct ExpandState {
    expanded: HashSet<String>,
    collapsed: HashSet<String>,
}

impl ExpandState {
    pub fn is_expanded(&self, id: &str, auto: &HashSet<String>) -> bool {
        (auto.contains(id) || self.expanded.contains(id)) && !self.collapsed.contains(id)
    }

    pub fn toggle(&mut self, id: &str, auto: &HashSet<String>) {
        if self.is_expanded(id, auto) {
            self.expanded.remove(id);
            self.collapsed.insert(id.to_string());
        } else {
            self.collapsed.remove(id);
            self.expanded.insert(id.to_string());
        }
    }
}

/// Groups that contain at least one host (over ALL hosts, not only filtered ones) plus their ancestors.
pub fn auto_expanded_groups(hosts: &[Host], groups: &[HostGroup]) -> HashSet<String> {
    let parent: HashMap<&str, Option<&str>> = groups.iter().map(|g| (g.id.as_str(), g.parent_id.as_deref())).collect();
    let mut out = HashSet::new();
    for host in hosts {
        let gid = host.group_id.as_deref().unwrap_or(DEFAULT_GROUP_ID);
        out.insert(gid.to_string());
        let mut cur = parent.get(gid).copied().flatten();
        let mut guard = 0;
        while let Some(p) = cur {
            if !out.insert(p.to_string()) || guard > 64 {
                break;
            }
            guard += 1;
            cur = parent.get(p).copied().flatten();
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Group { id: String, name: String, depth: usize, host_count: usize, expanded: bool, check: CheckState },
    Host { id: String, depth: usize, checked: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    None,
    Some,
    All,
}

/// Flatten the tree into visible rows. `default_group_name` names the implicit "ungrouped" group (always first).
pub fn build_rows(
    hosts: &[Host],
    groups: &[HostGroup],
    query: &str,
    default_group_name: &str,
    expand: &ExpandState,
    selected: &BTreeSet<String>,
) -> Vec<TreeRow> {
    let filtered: Vec<&Host> = hosts.iter().filter(|h| host_matches(h, query)).collect();
    let auto = auto_expanded_groups(hosts, groups);

    let mut hosts_by_group: HashMap<&str, Vec<&Host>> = HashMap::new();
    for h in &filtered {
        hosts_by_group.entry(h.group_id.as_deref().unwrap_or(DEFAULT_GROUP_ID)).or_default().push(h);
    }
    let mut groups_by_parent: HashMap<&str, Vec<&HostGroup>> = HashMap::new();
    for g in groups {
        groups_by_parent.entry(g.parent_id.as_deref().unwrap_or("")).or_default().push(g);
    }

    fn descendant_hosts<'a>(
        id: &str,
        hosts_by_group: &HashMap<&str, Vec<&'a Host>>,
        groups_by_parent: &HashMap<&str, Vec<&HostGroup>>,
        out: &mut Vec<&'a Host>,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        if let Some(hs) = hosts_by_group.get(id) {
            out.extend(hs.iter().copied());
        }
        if let Some(children) = groups_by_parent.get(id) {
            for c in children {
                descendant_hosts(&c.id, hosts_by_group, groups_by_parent, out, depth + 1);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        id: &str,
        name: &str,
        depth: usize,
        hosts_by_group: &HashMap<&str, Vec<&Host>>,
        groups_by_parent: &HashMap<&str, Vec<&HostGroup>>,
        auto: &HashSet<String>,
        expand: &ExpandState,
        selected: &BTreeSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        let direct = hosts_by_group.get(id).map(Vec::len).unwrap_or(0);
        let mut all = Vec::new();
        descendant_hosts(id, hosts_by_group, groups_by_parent, &mut all, 0);
        let picked = all.iter().filter(|h| selected.contains(&h.id)).count();
        let check = match (picked, all.len()) {
            (0, _) => CheckState::None,
            (p, t) if p == t => CheckState::All,
            _ => CheckState::Some,
        };
        let expanded = expand.is_expanded(id, auto);
        rows.push(TreeRow::Group { id: id.to_string(), name: name.to_string(), depth, host_count: direct, expanded, check });
        if !expanded || depth > 64 {
            return;
        }
        if let Some(children) = groups_by_parent.get(id) {
            for c in children {
                emit(&c.id, &c.name, depth + 1, hosts_by_group, groups_by_parent, auto, expand, selected, rows);
            }
        }
        if let Some(hs) = hosts_by_group.get(id) {
            for h in hs {
                rows.push(TreeRow::Host { id: h.id.clone(), depth: depth + 1, checked: selected.contains(&h.id) });
            }
        }
    }

    let mut rows = Vec::new();
    emit(DEFAULT_GROUP_ID, default_group_name, 0, &hosts_by_group, &groups_by_parent, &auto, expand, selected, &mut rows);
    for g in groups_by_parent.get("").into_iter().flatten() {
        emit(&g.id, &g.name, 0, &hosts_by_group, &groups_by_parent, &auto, expand, selected, &mut rows);
    }
    rows
}

/// Host ids under a group (visible after filtering), used when its checkbox is toggled.
pub fn group_host_ids(hosts: &[Host], groups: &[HostGroup], group_id: &str, query: &str) -> Vec<String> {
    let filtered: Vec<&Host> = hosts.iter().filter(|h| host_matches(h, query)).collect();
    let mut ids = Vec::new();
    let mut stack = vec![group_id.to_string()];
    let mut seen = HashSet::new();
    while let Some(gid) = stack.pop() {
        if !seen.insert(gid.clone()) {
            continue;
        }
        for h in &filtered {
            if h.group_id.as_deref().unwrap_or(DEFAULT_GROUP_ID) == gid {
                ids.push(h.id.clone());
            }
        }
        for g in groups.iter().filter(|g| g.parent_id.as_deref() == Some(gid.as_str())) {
            stack.push(g.id.clone());
        }
    }
    ids
}

/// Flattened group choices for pickers (depth-first, children after their parent) with the default group first.
/// Returns `(group id, depth)`; a group can be excluded together with its subtree (used when moving groups).
pub fn group_choices(groups: &[HostGroup], exclude_subtree_of: Option<&str>) -> Vec<(String, usize)> {
    fn walk(groups: &[HostGroup], parent: &str, depth: usize, exclude: Option<&str>, out: &mut Vec<(String, usize)>) {
        if depth > 64 {
            return;
        }
        for g in groups.iter().filter(|g| g.parent_id.as_deref().unwrap_or("") == parent) {
            if Some(g.id.as_str()) == exclude {
                continue;
            }
            out.push((g.id.clone(), depth));
            walk(groups, &g.id, depth + 1, exclude, out);
        }
    }
    let mut out = vec![(DEFAULT_GROUP_ID.to_string(), 0)];
    walk(groups, "", 0, exclude_subtree_of, &mut out);
    out
}

/// Selected Linux hosts only (broadcast terminal / batch upload targets).
pub fn selected_linux_host_ids(selected: &BTreeSet<String>, hosts: &[Host]) -> Vec<String> {
    hosts.iter().filter(|h| selected.contains(&h.id) && HostKind::of(h) == HostKind::Linux).map(|h| h.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str, name: &str, group: Option<&str>, os: &str) -> Host {
        Host {
            id: id.into(),
            name: name.into(),
            ip: format!("10.0.0.{}", id.len()),
            port: 22,
            auth_type: "password".into(),
            username: "root".into(),
            password: None,
            private_key: None,
            os: os.into(),
            tags: "[]".into(),
            group_id: group.map(Into::into),
            remark: String::new(),
            status: "unknown".into(),
            jump_chain: "[]".into(),
            rdp_settings: "{}".into(),
            proxy_settings: "{}".into(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn group(id: &str, name: &str, parent: Option<&str>) -> HostGroup {
        HostGroup { id: id.into(), name: name.into(), parent_id: parent.map(Into::into), sort_order: 0 }
    }

    fn sample() -> (Vec<Host>, Vec<HostGroup>) {
        (
            vec![
                host("h1", "alpha", None, "linux"),
                host("h22", "beta-web", Some("g-prod"), "linux"),
                host("h333", "gamma-db", Some("g-web"), "linux"),
                host("h4444", "win-1", Some("g-prod"), "windows"),
                host("h55555", "vnc-box", None, "vnc"),
            ],
            vec![group("g-prod", "prod", None), group("g-web", "web", Some("g-prod")), group("g-empty", "empty", None)],
        )
    }

    fn ids(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Group { id, depth, .. } => format!("G{depth}:{id}"),
                TreeRow::Host { id, depth, .. } => format!("H{depth}:{id}"),
            })
            .collect()
    }

    #[test]
    fn default_group_first_then_top_level_groups_children_before_hosts() {
        let (h, g) = sample();
        let rows = build_rows(&h, &g, "", "未分组", &ExpandState::default(), &BTreeSet::new());
        assert_eq!(
            ids(&rows),
            vec!["G0:__default__", "H1:h1", "H1:h55555", "G0:g-prod", "G1:g-web", "H2:h333", "H1:h22", "H1:h4444", "G0:g-empty"]
        );
    }

    #[test]
    fn auto_expand_only_groups_with_hosts_and_their_ancestors() {
        let (h, g) = sample();
        let auto = auto_expanded_groups(&h, &g);
        assert!(auto.contains(DEFAULT_GROUP_ID) && auto.contains("g-prod") && auto.contains("g-web"));
        assert!(!auto.contains("g-empty"));
        // g-web only has a host; its ancestor g-prod opens too even if prod itself had none
        let only_child = vec![host("x", "x", Some("g-web"), "linux")];
        let auto = auto_expanded_groups(&only_child, &g);
        assert!(auto.contains("g-prod"));
    }

    #[test]
    fn user_collapse_and_expand_override_auto_expansion() {
        let (h, g) = sample();
        let auto = auto_expanded_groups(&h, &g);
        let mut ex = ExpandState::default();
        ex.toggle("g-prod", &auto); // collapse
        let rows = build_rows(&h, &g, "", "d", &ex, &BTreeSet::new());
        assert!(!ids(&rows).iter().any(|r| r.contains("h22")));
        ex.toggle("g-prod", &auto); // expand again
        assert!(ids(&build_rows(&h, &g, "", "d", &ex, &BTreeSet::new())).iter().any(|r| r.contains("h22")));
        // an empty group the user opens manually stays expanded
        ex.toggle("g-empty", &auto);
        assert!(ex.is_expanded("g-empty", &auto));
    }

    #[test]
    fn search_filters_hosts_but_keeps_group_structure() {
        let (h, g) = sample();
        let rows = build_rows(&h, &g, "GAMMA", "d", &ExpandState::default(), &BTreeSet::new());
        let r = ids(&rows);
        assert!(r.contains(&"H2:h333".to_string()));
        assert!(!r.iter().any(|x| x.contains("h22") || x.contains(":h1")));
        assert!(r.contains(&"G0:g-empty".to_string()), "groups are never hidden by the search");
        assert!(host_matches(&h[0], "10.0.0.2") || host_matches(&h[0], "alpha"));
        assert!(host_matches(&h[0], "  "));
    }

    #[test]
    fn group_checkbox_is_tri_state_over_descendants() {
        let (h, g) = sample();
        let mut sel = BTreeSet::new();
        let state_of = |sel: &BTreeSet<String>, gid: &str| {
            build_rows(&h, &g, "", "d", &ExpandState::default(), sel)
                .into_iter()
                .find_map(|r| match r {
                    TreeRow::Group { id, check, .. } if id == gid => Some(check),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(state_of(&sel, "g-prod"), CheckState::None);
        sel.insert("h22".to_string());
        assert_eq!(state_of(&sel, "g-prod"), CheckState::Some);
        for id in group_host_ids(&h, &g, "g-prod", "") {
            sel.insert(id);
        }
        assert_eq!(state_of(&sel, "g-prod"), CheckState::All);
        assert_eq!(state_of(&sel, "g-web"), CheckState::All);
        assert_eq!(group_host_ids(&h, &g, "g-prod", "").len(), 3, "includes nested group hosts");
        assert_eq!(group_host_ids(&h, &g, "g-prod", "win").len(), 1, "respects the search filter");
    }

    #[test]
    fn group_choices_are_depth_first_with_default_first_and_can_exclude_a_subtree() {
        let (_, g) = sample();
        let all = group_choices(&g, None);
        assert_eq!(all, vec![(DEFAULT_GROUP_ID.to_string(), 0), ("g-prod".into(), 0), ("g-web".into(), 1), ("g-empty".into(), 0)]);
        let without = group_choices(&g, Some("g-prod"));
        assert_eq!(without.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), vec![DEFAULT_GROUP_ID, "g-empty"]);
    }

    #[test]
    fn host_kinds_and_linux_only_batch_targets() {
        let (mut h, _) = sample();
        assert_eq!(HostKind::of(&h[0]), HostKind::Linux);
        assert_eq!(HostKind::of(&h[3]), HostKind::Windows);
        assert_eq!(HostKind::of(&h[4]), HostKind::Vnc);
        h[1].rdp_settings = r#"{"protocol":"vnc"}"#.into();
        assert_eq!(HostKind::of(&h[1]), HostKind::Vnc, "rdp_settings.protocol=vnc wins");
        let sel: BTreeSet<String> = h.iter().map(|x| x.id.clone()).collect();
        assert_eq!(selected_linux_host_ids(&sel, &h), vec!["h1".to_string(), "h333".to_string()]);
    }
}
