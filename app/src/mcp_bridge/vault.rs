//! The host, group and script tools of the external MCP bridge ("vault" in Netcatty's words). Credentials never pass
//! through these tools: a host is created without a password or key (they are entered in OpsBatch) and an update
//! keeps the ones it has. Every change refreshes the sidebar and the library, so the window shows it at once.

use std::sync::Arc;

use gpui_kit::*;
use ops_ai::mcp_bridge::ToolOutcome;
use ops_library::library::{CustomScript, NewScript};
use ops_ssh::groups::HostGroup;
use ops_ssh::hosts::{Host, NewHost, UpdateHost};

use crate::sftp::store::blocking;
use serde_json::{json, Value};

pub(super) fn handles(tool: &str) -> bool {
    matches!(
        tool,
        "vault_list_hosts"
            | "vault_list_scripts"
            | "vault_list_groups"
            | "vault_get_script"
            | "vault_create_host"
            | "vault_update_host"
            | "vault_set_host_notes"
            | "vault_delete_host"
            | "vault_create_group"
            | "vault_update_group"
            | "vault_delete_group"
            | "vault_create_script"
            | "vault_update_script"
            | "vault_delete_script"
            | "vault_run_script"
            | "vault_get_script_run"
            | "vault_stop_script_run"
    )
}

/// The grantable capability of a changing tool (`None` for the reads).
pub(super) fn capability_of(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "vault_create_host" => "vault.hosts.create",
        "vault_update_host" => "vault.host.update",
        "vault_set_host_notes" => "vault.host.notes.set",
        "vault_delete_host" => "vault.host.delete",
        "vault_create_group" => "vault.group.create",
        "vault_update_group" => "vault.group.update",
        "vault_delete_group" => "vault.group.delete",
        "vault_create_script" => "vault.scripts.create",
        "vault_update_script" => "vault.scripts.update",
        "vault_delete_script" => "vault.scripts.delete",
        "vault_run_script" => "vault.scripts.run",
        "vault_stop_script_run" => "vault.scripts.run.stop",
        _ => return None,
    })
}

fn arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// What the record shows before the ids are resolved to names (never script text or notes).
pub(super) fn summary_of(tool: &str, args: &Value) -> String {
    match tool {
        "vault_create_host" => format!("{} ({})", arg(args, "name"), arg(args, "address")),
        "vault_create_group" | "vault_create_script" => arg(args, "name"),
        "vault_update_host" | "vault_set_host_notes" | "vault_delete_host" => arg(args, "host_id"),
        "vault_update_group" | "vault_delete_group" => arg(args, "group_id"),
        "vault_get_script" | "vault_update_script" | "vault_delete_script" | "vault_run_script" => {
            arg(args, "script_id")
        }
        "vault_get_script_run" | "vault_stop_script_run" => arg(args, "run_id"),
        _ => arg(args, "query"),
    }
}

/// The fields an update call changes, for the approval text.
fn changed_fields(args: &Value, skip: &[&str]) -> String {
    let mut keys: Vec<&str> = args
        .as_object()
        .map(|o| {
            o.keys()
                .map(String::as_str)
                .filter(|k| !skip.contains(k))
                .collect()
        })
        .unwrap_or_default();
    keys.sort_unstable();
    keys.join(", ")
}

/// The object of a call by name, so the person approving it does not have to match ids.
pub(super) async fn describe(tool: &str, args: &Value, ctx: &Arc<ops_ssh::Ctx>) -> Option<String> {
    let with_changes = |name: String, skip: &[&str]| {
        let fields = changed_fields(args, skip);
        if fields.is_empty() || !tool.contains("update") {
            name
        } else {
            format!("{name} · {fields}")
        }
    };
    match tool {
        "vault_update_host" | "vault_set_host_notes" | "vault_delete_host" => {
            let host = find_host(ctx, &arg(args, "host_id")).await.ok()?;
            let label = format!("{} ({}@{})", host.name, host.username, host.ip);
            Some(if tool == "vault_set_host_notes" {
                format!("{label} · notes")
            } else {
                with_changes(label, &["host_id"])
            })
        }
        "vault_update_group" | "vault_delete_group" => {
            let group = find_group(ctx, &arg(args, "group_id")).await.ok()?;
            Some(with_changes(group.name, &["group_id"]))
        }
        "vault_update_script" | "vault_delete_script" => {
            let script = find_script(ctx, &arg(args, "script_id")).await.ok()?;
            Some(with_changes(script.name, &["script_id"]))
        }
        "vault_run_script" => {
            let script = find_script(ctx, &arg(args, "script_id")).await.ok()?;
            let hosts = all_hosts(ctx).await.ok()?;
            let names: Vec<String> = host_ids_arg(args)
                .ok()?
                .iter()
                .filter_map(|id| hosts.iter().find(|h| &h.id == id))
                .map(|h| format!("{} ({})", h.name, h.ip))
                .collect();
            Some(format!(
                "{} [{}] → {}",
                script.name,
                script.risk,
                names.join(", ")
            ))
        }
        _ => None,
    }
}

/// `dry` runs every check of a changing call and stops before the first write (used to refuse a bad call before
/// anyone is asked to approve it).
pub(super) async fn run(
    tool: &str,
    args: &Value,
    ctx: &Arc<ops_ssh::Ctx>,
    cx: &mut AsyncApp,
    dry: bool,
) -> ToolOutcome {
    let result = match tool {
        "vault_list_hosts" => list_hosts(ctx, &arg(args, "query")).await,
        "vault_list_groups" => list_groups(ctx).await,
        "vault_list_scripts" => list_scripts(ctx, &arg(args, "query")).await,
        "vault_get_script" => get_script(ctx, &arg(args, "script_id")).await,
        "vault_create_host" => create_host(ctx, args, dry).await,
        "vault_update_host" => update_host(ctx, args, dry).await,
        "vault_set_host_notes" => {
            update_host(
                ctx,
                &json!({"host_id": arg(args, "host_id"), "remark": arg(args, "notes")}),
                dry,
            )
            .await
        }
        "vault_delete_host" => delete_host(ctx, &arg(args, "host_id"), dry).await,
        "vault_create_group" => create_group(ctx, args, dry).await,
        "vault_update_group" => update_group(ctx, args, dry).await,
        "vault_delete_group" => delete_group(ctx, &arg(args, "group_id"), dry).await,
        "vault_create_script" => create_script(ctx, args, dry).await,
        "vault_update_script" => update_script(ctx, args, dry).await,
        "vault_delete_script" => delete_script(ctx, &arg(args, "script_id"), dry).await,
        "vault_run_script" => run_script(ctx, args, dry).await,
        "vault_get_script_run" => run_json(ctx, &arg(args, "run_id")).await,
        "vault_stop_script_run" => stop_script_run(ctx, &arg(args, "run_id"), dry).await,
        other => Err(format!("Unknown tool: {other}")),
    };
    if capability_of(tool).is_some() && result.is_ok() && !dry {
        refresh(tool, cx);
    }
    match result {
        Ok(text) => ToolOutcome::ok(text),
        Err(e) => ToolOutcome::error(e),
    }
}

/// The sidebar and the library read the database once; tell them it changed.
fn refresh(tool: &str, cx: &mut AsyncApp) {
    let scripts = tool.contains("script");
    cx.update(|cx| {
        if !scripts {
            if let Some(assets) = crate::stores::assets::shared(cx) {
                assets.update(cx, |s, cx| s.reload(cx));
            }
        } else {
            crate::library::store::library_store(cx).update(cx, |l, cx| l.load_scripts(cx));
        }
    });
}

fn pretty(v: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| e.to_string())
}

fn contains_any(query: &str, fields: &[&str]) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || fields.iter().any(|f| f.to_lowercase().contains(&q))
}

/// Tags are stored as a JSON array; older rows may hold comma separated text.
fn parse_tags(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_else(|_| {
        raw.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    })
}

/// `tags` of a call as the stored JSON array (`None` when the call does not mention tags).
fn tags_arg(args: &Value) -> Result<Option<String>, String> {
    let Some(v) = args.get("tags") else {
        return Ok(None);
    };
    let list = v.as_array().ok_or("`tags` must be a list of strings.")?;
    let tags: Option<Vec<&str>> = list
        .iter()
        .map(|t| t.as_str().map(str::trim).filter(|t| !t.is_empty()))
        .collect();
    serde_json::to_string(&tags.ok_or("`tags` must be a list of non-empty strings.")?)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn port_arg(args: &Value) -> Result<Option<i32>, String> {
    match args.get("port") {
        None => Ok(None),
        Some(v) => v
            .as_i64()
            .filter(|p| (1..=65535).contains(p))
            .map(|p| Some(p as i32))
            .ok_or_else(|| "`port` must be a number from 1 to 65535.".to_string()),
    }
}

fn auth_arg(args: &Value) -> Result<Option<String>, String> {
    match args.get("auth_type").and_then(Value::as_str) {
        None => Ok(None),
        Some(a @ ("password" | "key")) => Ok(Some(a.to_string())),
        Some(other) => Err(format!(
            "`auth_type` must be password or key (got \"{other}\")."
        )),
    }
}

// ---------------------------------------------------------------------------
// Hosts
// ---------------------------------------------------------------------------

/// A host as the tools show it: no password, key, proxy or jump settings.
fn host_json(h: &Host, group: &str) -> Value {
    json!({"host_id": h.id, "name": h.name, "address": h.ip, "port": h.port, "username": h.username, "auth_type": h.auth_type, "system": h.os, "group": group, "group_id": h.group_id, "tags": parse_tags(&h.tags), "remark": h.remark})
}

async fn all_hosts(ctx: &Arc<ops_ssh::Ctx>) -> Result<Vec<Host>, String> {
    let c = ctx.clone();
    ops_core::rt::run(async move { ops_ssh::hosts::list_hosts(&c.db).await })
        .await
        .and_then(|r| r)
}

async fn all_groups(ctx: &Arc<ops_ssh::Ctx>) -> Result<Vec<HostGroup>, String> {
    let c = ctx.clone();
    ops_core::rt::run(async move { ops_ssh::groups::list_groups(&c.db).await })
        .await
        .and_then(|r| r)
}

async fn find_host(ctx: &Arc<ops_ssh::Ctx>, id: &str) -> Result<Host, String> {
    if id.is_empty() {
        return Err("`host_id` is required.".into());
    }
    all_hosts(ctx)
        .await?
        .into_iter()
        .find(|h| h.id == id)
        .ok_or_else(|| {
            format!("Unknown host_id: {id}. Use vault_list_hosts to see the saved hosts.")
        })
}

async fn find_group(ctx: &Arc<ops_ssh::Ctx>, id: &str) -> Result<HostGroup, String> {
    if id.is_empty() {
        return Err("`group_id` is required.".into());
    }
    all_groups(ctx)
        .await?
        .into_iter()
        .find(|g| g.id == id)
        .ok_or_else(|| format!("Unknown group_id: {id}. Use vault_list_groups to see the groups."))
}

async fn list_hosts(ctx: &Arc<ops_ssh::Ctx>, query: &str) -> Result<String, String> {
    let (hosts, groups) = (
        all_hosts(ctx).await?,
        all_groups(ctx).await.unwrap_or_default(),
    );
    let rows: Vec<Value> = hosts
        .iter()
        .map(|h| {
            (
                h,
                groups
                    .iter()
                    .find(|g| Some(&g.id) == h.group_id.as_ref())
                    .map(|g| g.name.clone())
                    .unwrap_or_default(),
            )
        })
        .filter(|(h, group)| contains_any(query, &[&h.name, &h.ip, group, &h.tags, &h.remark]))
        .map(|(h, group)| host_json(h, &group))
        .collect();
    pretty(&rows)
}

async fn create_host(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let (name, address) = (
        arg(args, "name").trim().to_string(),
        arg(args, "address").trim().to_string(),
    );
    if name.is_empty() || address.is_empty() {
        return Err("`name` and `address` are required.".into());
    }
    let group_id = Some(arg(args, "group_id")).filter(|g| !g.is_empty());
    if let Some(g) = &group_id {
        find_group(ctx, g).await?;
    }
    let host = NewHost {
        name,
        ip: address,
        port: port_arg(args)?,
        auth_type: auth_arg(args)?,
        username: args
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|u| !u.is_empty()),
        // credentials are entered in OpsBatch itself
        password: None,
        private_key: None,
        os: args
            .get("system")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty()),
        tags: tags_arg(args)?,
        group_id,
        remark: args
            .get("remark")
            .and_then(Value::as_str)
            .map(str::to_string),
        jump_chain: None,
        rdp_settings: None,
        proxy_settings: None,
    };
    if dry {
        return Ok(String::new());
    }
    let c = ctx.clone();
    let created = ops_core::rt::run(async move { ops_ssh::hosts::add_host(&c.db, host).await })
        .await
        .and_then(|r| r)?;
    pretty(&host_json(&created, ""))
}

async fn update_host(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let current = find_host(ctx, &arg(args, "host_id")).await?;
    let text = |key: &str, old: &str| -> Result<String, String> {
        match args.get(key) {
            None => Ok(old.to_string()),
            Some(v) => v
                .as_str()
                .map(str::trim)
                .filter(|s| key == "remark" || !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("`{key}` must be a non-empty string.")),
        }
    };
    let group_id = match args.get("group_id").and_then(Value::as_str) {
        None => current.group_id.clone(),
        Some("") => None,
        Some(g) => {
            find_group(ctx, g).await?;
            Some(g.to_string())
        }
    };
    let update = UpdateHost {
        id: current.id.clone(),
        name: text("name", &current.name)?,
        ip: if args.get("address").is_some() {
            text("address", &current.ip)?
        } else {
            current.ip.clone()
        },
        port: port_arg(args)?.unwrap_or(current.port),
        auth_type: auth_arg(args)?.unwrap_or_else(|| current.auth_type.clone()),
        username: text("username", &current.username)?,
        // None keeps the stored password and key
        password: None,
        private_key: None,
        os: if args.get("system").is_some() {
            text("system", &current.os)?
        } else {
            current.os.clone()
        },
        tags: tags_arg(args)?.unwrap_or_else(|| current.tags.clone()),
        group_id,
        remark: Some(if args.get("remark").is_some() {
            text("remark", &current.remark)?
        } else {
            current.remark.clone()
        }),
        jump_chain: Some(current.jump_chain.clone()),
        rdp_settings: Some(current.rdp_settings.clone()).filter(|s| !s.is_empty()),
        proxy_settings: Some(current.proxy_settings.clone()).filter(|s| !s.is_empty()),
    };
    if dry {
        return Ok(String::new());
    }
    let (core, registry) = (ctx.core.clone(), ctx.registry.clone());
    let saved =
        ops_core::rt::run(async move { ops_ssh::hosts::update_host(core, registry, update).await })
            .await
            .and_then(|r| r)?;
    pretty(&host_json(&saved, ""))
}

async fn delete_host(ctx: &Arc<ops_ssh::Ctx>, id: &str, dry: bool) -> Result<String, String> {
    let host = find_host(ctx, id).await?;
    if dry {
        return Ok(String::new());
    }
    let (core, registry, hid) = (ctx.core.clone(), ctx.registry.clone(), host.id.clone());
    ops_core::rt::run(async move { ops_ssh::hosts::delete_host(&core, registry, hid).await })
        .await
        .and_then(|r| r)?;
    Ok(format!("Deleted host {} ({})", host.name, host.id))
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

fn group_json(g: &HostGroup) -> Value {
    json!({"group_id": g.id, "name": g.name, "parent_id": g.parent_id})
}

async fn list_groups(ctx: &Arc<ops_ssh::Ctx>) -> Result<String, String> {
    pretty(
        &all_groups(ctx)
            .await?
            .iter()
            .map(group_json)
            .collect::<Vec<_>>(),
    )
}

async fn create_group(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let name = arg(args, "name").trim().to_string();
    if name.is_empty() {
        return Err("`name` is required.".into());
    }
    let parent = Some(arg(args, "parent_id")).filter(|p| !p.is_empty());
    if let Some(p) = &parent {
        find_group(ctx, p).await?;
    }
    if dry {
        return Ok(String::new());
    }
    let c = ctx.clone();
    let group =
        ops_core::rt::run(async move { ops_ssh::groups::add_group(&c.db, name, parent).await })
            .await
            .and_then(|r| r)?;
    pretty(&group_json(&group))
}

async fn update_group(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let mut group = find_group(ctx, &arg(args, "group_id")).await?;
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        if name.trim().is_empty() {
            return Err("`name` must not be empty.".into());
        }
        group.name = name.trim().to_string();
    }
    if let Some(parent) = args.get("parent_id").and_then(Value::as_str) {
        if parent.is_empty() {
            group.parent_id = None;
        } else {
            // a group cannot sit below itself
            let all = all_groups(ctx).await?;
            let mut cursor = Some(parent.to_string());
            while let Some(id) = cursor {
                if id == group.id {
                    return Err("A group cannot be moved below itself.".into());
                }
                cursor = all
                    .iter()
                    .find(|g| g.id == id)
                    .and_then(|g| g.parent_id.clone());
            }
            find_group(ctx, parent).await?;
            group.parent_id = Some(parent.to_string());
        }
    }
    if dry {
        return Ok(String::new());
    }
    let c = ctx.clone();
    let saved = ops_core::rt::run(async move { ops_ssh::groups::update_group(&c.db, group).await })
        .await
        .and_then(|r| r)?;
    pretty(&group_json(&saved))
}

async fn delete_group(ctx: &Arc<ops_ssh::Ctx>, id: &str, dry: bool) -> Result<String, String> {
    let group = find_group(ctx, id).await?;
    if dry {
        return Ok(String::new());
    }
    let (c, gid) = (ctx.clone(), group.id.clone());
    ops_core::rt::run(async move { ops_ssh::groups::delete_group(&c.db, gid).await })
        .await
        .and_then(|r| r)?;
    Ok(format!(
        "Deleted group {} ({}); its hosts are ungrouped now",
        group.name, group.id
    ))
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------

fn script_row(s: &CustomScript) -> Value {
    json!({"script_id": s.id, "name": s.name, "language": s.language, "category": s.category, "risk": s.risk, "platform": s.platform, "description": s.description, "tags": parse_tags(&s.tags), "builtin": s.is_builtin})
}

async fn all_scripts(ctx: &Arc<ops_ssh::Ctx>) -> Result<Vec<CustomScript>, String> {
    let c = ctx.clone();
    ops_core::rt::run(async move { ops_library::library::list_scripts(&c.db).await })
        .await
        .and_then(|r| r)
}

async fn find_script(ctx: &Arc<ops_ssh::Ctx>, id: &str) -> Result<CustomScript, String> {
    if id.is_empty() {
        return Err("`script_id` is required.".into());
    }
    all_scripts(ctx)
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| {
            format!("Unknown script_id: {id}. Use vault_list_scripts to see the library scripts.")
        })
}

async fn list_scripts(ctx: &Arc<ops_ssh::Ctx>, query: &str) -> Result<String, String> {
    let rows: Vec<Value> = all_scripts(ctx)
        .await?
        .iter()
        .filter(|s| contains_any(query, &[&s.name, &s.category, &s.tags, &s.description]))
        .map(script_row)
        .collect();
    pretty(&rows)
}

async fn get_script(ctx: &Arc<ops_ssh::Ctx>, id: &str) -> Result<String, String> {
    let s = find_script(ctx, id).await?;
    let mut v = script_row(&s);
    v["content"] = json!(s.content);
    pretty(&v)
}

fn risk_arg(args: &Value) -> Result<Option<String>, String> {
    match args.get("risk").and_then(Value::as_str) {
        None => Ok(None),
        Some(r @ ("low" | "medium" | "high")) => Ok(Some(r.to_string())),
        Some(other) => Err(format!(
            "`risk` must be low, medium or high (got \"{other}\")."
        )),
    }
}

async fn create_script(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let name = arg(args, "name").trim().to_string();
    if name.is_empty() {
        return Err("`name` is required.".into());
    }
    let Some(content) = args.get("content").and_then(Value::as_str) else {
        return Err("`content` is required.".into());
    };
    let opt = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|v| !v.is_empty())
    };
    let script = NewScript {
        name,
        language: opt("language"),
        category: opt("category"),
        tags: tags_arg(args)?,
        risk: risk_arg(args)?,
        description: opt("description"),
        content: Some(content.to_string()),
        parameters: None,
        url: None,
        platform: opt("platform"),
    };
    if dry {
        return Ok(String::new());
    }
    let c = ctx.clone();
    let id =
        ops_core::rt::run(async move { ops_library::library::add_script(&c.db, script).await })
            .await
            .and_then(|r| r)?;
    pretty(&script_row(&find_script(ctx, &id).await?))
}

async fn update_script(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let mut script = find_script(ctx, &arg(args, "script_id")).await?;
    if script.is_builtin {
        return Err(
            "Built-in scripts cannot be changed. Create a copy with vault_create_script instead."
                .into(),
        );
    }
    let set = |key: &str, field: &mut String| -> Result<(), String> {
        if let Some(v) = args.get(key) {
            *field = v
                .as_str()
                .ok_or_else(|| format!("`{key}` must be a string."))?
                .to_string();
        }
        Ok(())
    };
    set("name", &mut script.name)?;
    if script.name.trim().is_empty() {
        return Err("`name` must not be empty.".into());
    }
    set("content", &mut script.content)?;
    set("language", &mut script.language)?;
    set("category", &mut script.category)?;
    set("description", &mut script.description)?;
    set("platform", &mut script.platform)?;
    if let Some(risk) = risk_arg(args)? {
        script.risk = risk;
    }
    if let Some(tags) = tags_arg(args)? {
        script.tags = tags;
    }
    if dry {
        return Ok(String::new());
    }
    let (c, copy) = (ctx.clone(), script.clone());
    ops_core::rt::run(async move { ops_library::library::update_script(&c.db, copy).await })
        .await
        .and_then(|r| r)?;
    pretty(&script_row(&find_script(ctx, &script.id).await?))
}

async fn delete_script(ctx: &Arc<ops_ssh::Ctx>, id: &str, dry: bool) -> Result<String, String> {
    let script = find_script(ctx, id).await?;
    if script.is_builtin {
        return Err("Built-in scripts cannot be deleted.".into());
    }
    if dry {
        return Ok(String::new());
    }
    let (c, sid) = (ctx.clone(), script.id.clone());
    ops_core::rt::run(async move { ops_library::library::delete_script(&c.db, sid).await })
        .await
        .and_then(|r| r)?;
    Ok(format!("Deleted script {} ({})", script.name, script.id))
}

// ---------------------------------------------------------------------------
// Script runs (the batch execution of the Commands page, so a run shows up in its history too)
// ---------------------------------------------------------------------------

const RUN_MAX_HOSTS: usize = 20;
const RUN_CONCURRENCY: usize = 5;
const RUN_OUTPUT_CHARS: usize = 4000;
const DEFAULT_RUN_TIMEOUT: u64 = 30;
const DEFAULT_RUN_WAIT: u64 = 60;

fn host_ids_arg(args: &Value) -> Result<Vec<String>, String> {
    let list = args
        .get("host_ids")
        .and_then(Value::as_array)
        .ok_or("`host_ids` must be a list of host_id values.")?;
    let mut ids: Vec<String> = Vec::new();
    for v in list {
        let id = v
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("`host_ids` must hold non-empty strings.")?;
        if !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
    }
    match ids.len() {
        0 => Err("`host_ids` needs at least one host.".into()),
        n if n > RUN_MAX_HOSTS => Err(format!("At most {RUN_MAX_HOSTS} hosts per run (got {n}).")),
        _ => Ok(ids),
    }
}

fn seconds_arg(args: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match args.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("`{key}` must be a number from {min} to {max}.")),
    }
}

async fn run_script(ctx: &Arc<ops_ssh::Ctx>, args: &Value, dry: bool) -> Result<String, String> {
    let script = find_script(ctx, &arg(args, "script_id")).await?;
    if !matches!(
        script.language.to_lowercase().as_str(),
        "shell" | "bash" | "sh"
    ) {
        return Err(format!(
            "Only shell scripts can be run through MCP (this one is {}).",
            script.language
        ));
    }
    if !script.url.trim().is_empty() {
        return Err("Scripts that download from a URL cannot be run through MCP.".into());
    }
    if script.content.trim().is_empty() {
        return Err("The script is empty.".into());
    }
    let ids = host_ids_arg(args)?;
    let hosts = all_hosts(ctx).await?;
    if let Some(unknown) = ids.iter().find(|id| !hosts.iter().any(|h| &h.id == *id)) {
        return Err(format!(
            "Unknown host_id: {unknown}. Use vault_list_hosts to see the saved hosts."
        ));
    }
    let timeout = seconds_arg(args, "timeout_seconds", DEFAULT_RUN_TIMEOUT, 1, 600)?;
    let wait = seconds_arg(args, "wait_seconds", DEFAULT_RUN_WAIT, 0, 600)?;
    // the same policy as a command typed by the agent: observer mode and the blocklist refuse whatever the mode
    let (c, content) = (ctx.clone(), script.content.clone());
    let assessment =
        ops_core::rt::run(async move { ops_ai::client::ai_assess_action(&c.db, content) })
            .await
            .and_then(|r| r)
            .map_err(|e| format!("Refused: the safety policy could not be evaluated ({e})."))?;
    if assessment.decision == ops_ai::client::AiActionDecision::Block {
        return Err(format!("Refused by OpsBatch: {}", assessment.reason));
    }
    if dry {
        return Ok(String::new());
    }
    let task_id = uuid::Uuid::new_v4().to_string();
    let done = ctx.events.subscribe(format!("exec:{task_id}:done"));
    let (c, tid, content, hosts_to_run) = (
        ctx.clone(),
        task_id.clone(),
        script.content.clone(),
        ids.clone(),
    );
    let concurrency = hosts_to_run.len().min(RUN_CONCURRENCY) as u32;
    blocking(move || {
        ops_ssh::execution::execute_command(
            c.clone(),
            &c.db,
            tid,
            hosts_to_run,
            content,
            concurrency,
            timeout,
            None,
        )
    })
    .await
    .map_err(|e| format!("Could not start the run: {e}"))?;
    if wait > 0 {
        let _ = ops_core::rt::run(async move {
            tokio::time::timeout(std::time::Duration::from_secs(wait), done.recv_async())
                .await
                .is_ok()
        })
        .await;
    }
    run_json(ctx, &task_id).await
}

/// The state of a run: the history row (running until it has a completion time) and the hosts that reported so far.
async fn run_json(ctx: &Arc<ops_ssh::Ctx>, run_id: &str) -> Result<String, String> {
    if run_id.is_empty() {
        return Err("`run_id` is required.".into());
    }
    let c = ctx.clone();
    let history =
        ops_core::rt::run(async move { ops_ssh::execution::list_execution_history(&c.db).await })
            .await
            .and_then(|r| r)?;
    let Some(row) = history.into_iter().find(|h| h.id == run_id) else {
        return Err(format!("Unknown run_id: {run_id}."));
    };
    let (c, id) = (ctx.clone(), run_id.to_string());
    let details =
        ops_core::rt::run(async move { ops_ssh::execution::get_execution_detail(&c.db, id).await })
            .await
            .and_then(|r| r)?;
    let running = row.completed_at.trim().is_empty();
    let hosts: Vec<Value> = details
        .iter()
        .map(|d| json!({"host_id": d.host_id, "name": d.host_name, "status": d.status, "exit_code": d.exit_code, "duration_ms": d.duration, "output": super::tail(&super::strip_ansi(&d.output), RUN_OUTPUT_CHARS)}))
        .collect();
    pretty(&json!({
        "run_id": row.id,
        "status": if running { "running" } else { "completed" },
        "started_at": row.started_at,
        "completed_at": row.completed_at,
        "hosts_total": row.host_count,
        "hosts_reported": details.len(),
        "success": row.success_count,
        "failed": row.fail_count,
        "hosts": hosts,
    }))
}

async fn stop_script_run(
    ctx: &Arc<ops_ssh::Ctx>,
    run_id: &str,
    dry: bool,
) -> Result<String, String> {
    let state: Value =
        serde_json::from_str(&run_json(ctx, run_id).await?).map_err(|e| e.to_string())?;
    if state["status"] != "running" {
        return Err("The run has already finished.".into());
    }
    if dry {
        return Ok(String::new());
    }
    let (c, id) = (ctx.clone(), run_id.to_string());
    ops_core::rt::run(async move { ops_ssh::execution::cancel_execution(&c.db, id).await })
        .await
        .and_then(|r| r)
        .map_err(|e| format!("Could not stop the run: {e}"))?;
    Ok(format!("Stop requested for run {run_id}: hosts that have not started are skipped; a script already running on a host finishes on its own."))
}

#[cfg(test)]
mod tests {
    use super::{
        capability_of, contains_any, handles, host_ids_arg, parse_tags, seconds_arg, summary_of,
        tags_arg,
    };
    use serde_json::json;

    #[test]
    fn the_query_matches_any_field_ignoring_case() {
        assert!(contains_any("PROD", &["web-prod-1", "10.0.0.1"]));
        assert!(contains_any("", &["x"]));
        assert!(!contains_any("db", &["web", "10.0.0.1"]));
    }

    #[test]
    fn tags_round_trip_between_json_and_text() {
        assert_eq!(parse_tags(r#"["prod","web"]"#), vec!["prod", "web"]);
        assert_eq!(parse_tags("prod, web"), vec!["prod", "web"]);
        assert!(parse_tags("[]").is_empty());
        assert_eq!(
            tags_arg(&json!({"tags": [" a ", "b"]})).unwrap().as_deref(),
            Some(r#"["a","b"]"#)
        );
        assert_eq!(tags_arg(&json!({})).unwrap(), None);
        assert!(tags_arg(&json!({"tags": "a"})).is_err());
        assert!(tags_arg(&json!({"tags": [1]})).is_err());
        assert!(tags_arg(&json!({"tags": [""]})).is_err());
    }

    #[test]
    fn run_arguments_are_checked() {
        assert_eq!(
            host_ids_arg(&json!({"host_ids": ["a", " b ", "a"]})).unwrap(),
            vec!["a", "b"]
        );
        assert!(host_ids_arg(&json!({"host_ids": []}))
            .unwrap_err()
            .contains("at least one"));
        assert!(host_ids_arg(&json!({})).is_err());
        assert!(host_ids_arg(&json!({"host_ids": [1]})).is_err());
        let many: Vec<String> = (0..21).map(|i| format!("h{i}")).collect();
        assert!(host_ids_arg(&json!({"host_ids": many}))
            .unwrap_err()
            .contains("At most 20"));
        assert_eq!(
            seconds_arg(&json!({}), "timeout_seconds", 120, 1, 3600),
            Ok(120)
        );
        assert_eq!(
            seconds_arg(&json!({"wait_seconds": 0}), "wait_seconds", 60, 0, 600),
            Ok(0)
        );
        assert!(seconds_arg(
            &json!({"timeout_seconds": 0}),
            "timeout_seconds",
            120,
            1,
            3600
        )
        .is_err());
        assert!(seconds_arg(
            &json!({"timeout_seconds": "5"}),
            "timeout_seconds",
            120,
            1,
            3600
        )
        .is_err());
        assert!(seconds_arg(&json!({"wait_seconds": 601}), "wait_seconds", 60, 0, 600).is_err());
    }

    #[test]
    fn writes_map_to_grantable_capabilities_and_hide_text() {
        for tool in [
            "vault_create_host",
            "vault_update_host",
            "vault_set_host_notes",
            "vault_delete_host",
            "vault_create_group",
            "vault_update_group",
            "vault_delete_group",
            "vault_create_script",
            "vault_update_script",
            "vault_delete_script",
            "vault_run_script",
            "vault_stop_script_run",
        ] {
            let cap = capability_of(tool).unwrap_or_else(|| panic!("{tool}"));
            assert!(
                ops_ai::prefs::safety::GRANTABLE_CAPABILITIES.contains(&cap),
                "{cap}"
            );
            assert!(handles(tool));
        }
        for tool in [
            "vault_list_hosts",
            "vault_list_groups",
            "vault_list_scripts",
            "vault_get_script",
            "vault_get_script_run",
        ] {
            assert!(capability_of(tool).is_none() && handles(tool), "{tool}");
        }
        // the approval of a script write shows its name, never its text; notes never show either
        assert_eq!(
            summary_of(
                "vault_create_script",
                &json!({"name": "Restart", "content": "systemctl restart x"})
            ),
            "Restart"
        );
        assert_eq!(
            summary_of(
                "vault_set_host_notes",
                &json!({"host_id": "h1", "notes": "secret note"})
            ),
            "h1"
        );
    }
}
