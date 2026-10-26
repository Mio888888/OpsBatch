//! Hosts, groups and the multi-selection used by every batch action (port of `stores/assets.ts`, read side).

use std::collections::BTreeSet;

use gpui_kit::{Context, EventEmitter, Task};
use ops_ssh::hosts::{NewHost, UpdateHost};
use ops_ssh::groups::HostGroup;
use ops_ssh::hosts::Host;

use crate::asset_tree::{group_host_ids, selected_linux_host_ids};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetsEvent {
    Loaded,
    SelectionChanged,
}

pub struct AssetsStore {
    pub hosts: Vec<Host>,
    pub groups: Vec<HostGroup>,
    pub selected: BTreeSet<String>,
    pub loading: bool,
    pub error: Option<String>,
    /// User-chosen name of the implicit "ungrouped" group (empty = localised default).
    pub default_group_name: String,
    /// Host ids of the latest connections, newest first (persisted in the general settings).
    pub recent: Vec<String>,
    load_seq: u64,
    /// (sequence of the latest recent-list save issued, sequence of the latest one written): saves may overtake each
    /// other on the runtime, so an older list never overwrites a newer one.
    recent_saves: (u64, std::sync::Arc<tokio::sync::Mutex<u64>>),
}

impl EventEmitter<AssetsEvent> for AssetsStore {}

const DEFAULT_GROUP_NAME_KEY: &str = "defaultGroupName";
const RECENT_HOSTS_KEY: &str = "recentHosts";
pub const RECENT_HOSTS_MAX: usize = 8;

/// Move `id` to the front of the recent list (no duplicates, at most `max` entries).
pub fn push_recent(list: &mut Vec<String>, id: &str, max: usize) {
    list.retain(|x| x != id);
    list.insert(0, id.to_string());
    list.truncate(max);
}

fn parse_recent(raw: Option<&String>) -> Vec<String> {
    let mut list: Vec<String> = raw.and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    list.retain(|id| seen.insert(id.clone()));
    list.truncate(RECENT_HOSTS_MAX);
    list
}

impl AssetsStore {
    pub fn new() -> Self {
        Self {
            hosts: Vec::new(),
            groups: Vec::new(),
            selected: BTreeSet::new(),
            loading: false,
            error: None,
            default_group_name: String::new(),
            recent: Vec::new(),
            load_seq: 0,
            recent_saves: (0, std::sync::Arc::new(tokio::sync::Mutex::new(0))),
        }
    }

    /// (Re)load hosts, groups and the default group name from the database. Stale responses are ignored.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let ctx = crate::services::ctx(cx);
        self.load_seq += 1;
        let seq = self.load_seq;
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(async move {
                let hosts = ops_ssh::hosts::list_hosts(&ctx.db).await?;
                let groups = ops_ssh::groups::list_groups(&ctx.db).await?;
                let settings = ops_core::settings::get_general_settings(&ctx.db).await.unwrap_or_default();
                Ok::<_, String>((hosts, groups, settings.get(DEFAULT_GROUP_NAME_KEY).cloned().unwrap_or_default(), parse_recent(settings.get(RECENT_HOSTS_KEY))))
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                if seq != this.load_seq {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((hosts, groups, default_name, recent)) => {
                        // Drop selections of hosts that no longer exist.
                        let live: BTreeSet<&str> = hosts.iter().map(|h| h.id.as_str()).collect();
                        this.selected.retain(|id| live.contains(id.as_str()));
                        this.hosts = hosts;
                        this.groups = groups;
                        this.default_group_name = default_name;
                        this.recent = recent;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.emit(AssetsEvent::Loaded);
                cx.notify();
            });
        })
        .detach();
    }

    /// Recent connections that still exist, newest first.
    pub fn recent_hosts(&self) -> Vec<&Host> {
        self.recent.iter().filter_map(|id| self.host(id)).collect()
    }

    /// Remember that a session to `id` was just opened and persist the list.
    pub fn record_recent(&mut self, id: &str, cx: &mut Context<Self>) {
        push_recent(&mut self.recent, id, RECENT_HOSTS_MAX);
        cx.notify();
        let ctx = crate::services::ctx(cx);
        let value = serde_json::to_string(&self.recent).unwrap_or_else(|_| "[]".into());
        self.recent_saves.0 += 1;
        let (seq, written) = (self.recent_saves.0, self.recent_saves.1.clone());
        cx.spawn(async move |_, _| {
            let _ = ops_core::rt::run(async move {
                let mut last = written.lock().await;
                if seq > *last {
                    *last = seq;
                    let _ = ops_core::settings::save_general_settings(&ctx.db, std::collections::HashMap::from([(RECENT_HOSTS_KEY.to_string(), value)])).await;
                }
            })
            .await;
        })
        .detach();
    }

    // ---- writes: every operation returns a task resolving to the backend result, then reloads the lists ----

    fn write<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        op: impl FnOnce(std::sync::Arc<ops_ssh::Ctx>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>>,
    ) -> Task<Result<T, String>> {
        let ctx = crate::services::ctx(cx);
        let fut = op(ctx);
        cx.spawn(async move |this, cx| {
            let result = ops_core::rt::run(fut).await.and_then(|r| r);
            if result.is_ok() {
                let _ = this.update(cx, |this, cx| this.reload(cx));
            }
            result
        })
    }

    pub fn add_host(&mut self, host: NewHost, cx: &mut Context<Self>) -> Task<Result<Host, String>> {
        self.write(cx, move |ctx| Box::pin(async move { ops_ssh::hosts::add_host(&ctx.db, host).await }))
    }

    pub fn update_host(&mut self, host: UpdateHost, cx: &mut Context<Self>) -> Task<Result<Host, String>> {
        self.write(cx, move |ctx| {
            Box::pin(async move { ops_ssh::hosts::update_host(ctx.core.clone(), ctx.registry.clone(), host).await })
        })
    }

    pub fn delete_host(&mut self, id: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.selected.remove(&id);
        self.write(cx, move |ctx| Box::pin(async move { ops_ssh::hosts::delete_host(&ctx.core, ctx.registry.clone(), id).await }))
    }

    /// Move a host to another group (`None` = ungrouped). No-op when it is already there.
    pub fn move_host(&mut self, host_id: &str, group_id: Option<String>, cx: &mut Context<Self>) -> Option<Task<Result<Host, String>>> {
        let host = self.host(host_id)?.clone();
        if host.group_id == group_id {
            return None;
        }
        Some(self.update_host(update_from_host(&host, group_id), cx))
    }

    pub fn add_group(&mut self, name: String, parent_id: Option<String>, cx: &mut Context<Self>) -> Task<Result<HostGroup, String>> {
        self.write(cx, move |ctx| Box::pin(async move { ops_ssh::groups::add_group(&ctx.db, name, parent_id).await }))
    }

    pub fn rename_group(&mut self, id: &str, name: String, cx: &mut Context<Self>) -> Option<Task<Result<HostGroup, String>>> {
        let group = HostGroup { name, ..self.groups.iter().find(|g| g.id == id)?.clone() };
        Some(self.write(cx, move |ctx| Box::pin(async move { ops_ssh::groups::update_group(&ctx.db, group).await })))
    }

    pub fn delete_group(&mut self, id: String, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        self.write(cx, move |ctx| Box::pin(async move { ops_ssh::groups::delete_group(&ctx.db, id).await }))
    }

    /// The implicit "ungrouped" group can only be renamed locally (stored in general settings).
    pub fn set_default_group_name(&mut self, name: String, cx: &mut Context<Self>) {
        self.default_group_name = name.clone();
        cx.notify();
        let ctx = crate::services::ctx(cx);
        ops_core::rt::spawn(async move {
            let mut m = std::collections::HashMap::new();
            m.insert(DEFAULT_GROUP_NAME_KEY.to_string(), name);
            if let Err(error) = ops_core::settings::save_general_settings(&ctx.db, m).await {
                log::warn!("could not save default group name: {error}");
            }
        });
    }

    pub fn host(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }

    pub fn set_selected(&mut self, ids: impl IntoIterator<Item = String>, cx: &mut Context<Self>) {
        self.selected = ids.into_iter().collect();
        cx.emit(AssetsEvent::SelectionChanged);
        cx.notify();
    }

    pub fn toggle_host(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.selected.remove(id) {
            self.selected.insert(id.to_string());
        }
        cx.emit(AssetsEvent::SelectionChanged);
        cx.notify();
    }

    /// Toggle every (filtered) host below a group: all selected -> clear them, otherwise select them all.
    pub fn toggle_group(&mut self, group_id: &str, query: &str, cx: &mut Context<Self>) {
        let ids = group_host_ids(&self.hosts, &self.groups, group_id, query);
        if ids.is_empty() {
            return;
        }
        if ids.iter().all(|id| self.selected.contains(id)) {
            for id in &ids {
                self.selected.remove(id);
            }
        } else {
            self.selected.extend(ids);
        }
        cx.emit(AssetsEvent::SelectionChanged);
        cx.notify();
    }

    pub fn selected_linux_host_ids(&self) -> Vec<String> {
        selected_linux_host_ids(&self.selected, &self.hosts)
    }
}

impl Default for AssetsStore {
    fn default() -> Self {
        Self::new()
    }
}

/// An `UpdateHost` that changes only the group, passing the stored values (secrets as placeholders = "keep").
pub fn update_from_host(host: &Host, group_id: Option<String>) -> UpdateHost {
    UpdateHost {
        id: host.id.clone(),
        name: host.name.clone(),
        ip: host.ip.clone(),
        port: host.port,
        auth_type: host.auth_type.clone(),
        username: host.username.clone(),
        password: host.password.clone(),
        private_key: host.private_key.clone(),
        os: host.os.clone(),
        tags: host.tags.clone(),
        group_id,
        remark: Some(host.remark.clone()),
        jump_chain: Some(host.jump_chain.clone()),
        rdp_settings: Some(host.rdp_settings.clone()),
        proxy_settings: Some(host.proxy_settings.clone()),
    }
}

#[cfg(test)]
mod recent_tests {
    use super::*;

    #[test]
    fn newest_first_without_duplicates_and_capped() {
        let mut l = Vec::new();
        for id in ["a", "b", "c", "a"] {
            push_recent(&mut l, id, 3);
        }
        assert_eq!(l, ["a", "c", "b"]);
        push_recent(&mut l, "d", 3);
        assert_eq!(l, ["d", "a", "c"]);
    }

    #[test]
    fn stored_list_is_tolerant() {
        assert!(parse_recent(None).is_empty());
        assert!(parse_recent(Some(&"nonsense".to_string())).is_empty());
        assert_eq!(parse_recent(Some(&r#"["x","y","x"]"#.to_string())), ["x", "y"]);
    }
}
