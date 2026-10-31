//! Service-level tests that need only the database (no SSH): hosts, groups, tags, CSV, secrets, jump topology.

use ops_core::keychain;
use ops_core::security::SECRET_PLACEHOLDER;
use ops_core::Core;
use ops_ssh::hosts::{self, NewHost};
use ops_ssh::{groups, import, proxyjump, tags};

/// The registry owns a tokio runtime; dropping it inside an async test would panic, so keep one ref alive forever.
fn registry() -> std::sync::Arc<ops_ssh::ssh::SshConnectionRegistry> {
    let r = std::sync::Arc::new(ops_ssh::ssh::SshConnectionRegistry::new());
    std::mem::forget(r.clone());
    r
}

fn new_host(name: &str, ip: &str) -> NewHost {
    NewHost {
        name: name.into(),
        ip: ip.into(),
        port: Some(22),
        auth_type: Some("password".into()),
        username: Some("root".into()),
        password: Some("s3cret".into()),
        private_key: None,
        os: Some("linux".into()),
        tags: Some("[]".into()),
        group_id: None,
        remark: Some("".into()),
        jump_chain: None,
        rdp_settings: None,
        proxy_settings: None,
    }
}

#[tokio::test]
async fn host_crud_keeps_secrets_out_of_the_database() {
    let core = Core::open_temp("hosts").unwrap();
    let h = hosts::add_host(&core.db, new_host("web-1", "10.0.0.1"))
        .await
        .unwrap();
    assert_eq!(
        h.password.as_deref(),
        Some(SECRET_PLACEHOLDER),
        "password must be a placeholder in the row"
    );
    assert_eq!(keychain::get_host_password(&h.id).unwrap(), "s3cret");

    let listed = hosts::list_hosts(&core.db).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].password.as_deref(), Some(SECRET_PLACEHOLDER));

    // Raw column never holds the plaintext.
    let raw: Option<String> = core
        .db
        .pool
        .get()
        .unwrap()
        .query_row("SELECT password FROM hosts WHERE id=?1", [&h.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_ne!(raw.as_deref(), Some("s3cret"));

    // Updating with the placeholder keeps the stored secret; a new value replaces it.
    let registry = registry();
    let upd = |password: Option<&str>, name: &str| hosts::UpdateHost {
        id: h.id.clone(),
        name: name.into(),
        ip: "10.0.0.1".into(),
        port: 22,
        auth_type: "password".into(),
        username: "root".into(),
        password: password.map(Into::into),
        private_key: None,
        os: "linux".into(),
        tags: "[]".into(),
        group_id: None,
        remark: Some("".into()),
        jump_chain: None,
        rdp_settings: None,
        proxy_settings: None,
    };
    hosts::update_host(
        core.clone(),
        registry.clone(),
        upd(Some(SECRET_PLACEHOLDER), "web-1b"),
    )
    .await
    .unwrap();
    assert_eq!(keychain::get_host_password(&h.id).unwrap(), "s3cret");
    hosts::update_host(core.clone(), registry.clone(), upd(Some("n3w"), "web-1b"))
        .await
        .unwrap();
    assert_eq!(keychain::get_host_password(&h.id).unwrap(), "n3w");
    assert_eq!(hosts::list_hosts(&core.db).await.unwrap()[0].name, "web-1b");

    hosts::delete_host(&core, registry, h.id.clone())
        .await
        .unwrap();
    assert!(hosts::list_hosts(&core.db).await.unwrap().is_empty());
    assert!(
        keychain::get_host_password(&h.id).is_err(),
        "secret removed with the host"
    );
}

#[tokio::test]
async fn groups_nest_and_deleting_a_group_ungroups_hosts() {
    let core = Core::open_temp("groups").unwrap();
    let parent = groups::add_group(&core.db, "prod".into(), None)
        .await
        .unwrap();
    let child = groups::add_group(&core.db, "prod-web".into(), Some(parent.id.clone()))
        .await
        .unwrap();
    assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
    let mut h = new_host("a", "10.0.0.2");
    h.group_id = Some(parent.id.clone());
    let h = hosts::add_host(&core.db, h).await.unwrap();
    assert_eq!(h.group_id.as_deref(), Some(parent.id.as_str()));

    let renamed = groups::update_group(
        &core.db,
        ops_ssh::groups::HostGroup {
            name: "production".into(),
            ..parent.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(renamed.name, "production");

    groups::delete_group(&core.db, parent.id.clone())
        .await
        .unwrap();
    let after = hosts::list_hosts(&core.db).await.unwrap();
    assert_eq!(after[0].group_id, None);
    let all = groups::list_groups(&core.db).await.unwrap();
    assert!(all.iter().all(|g| g.id != parent.id));
}

#[tokio::test]
async fn tags_crud_and_unique_names() {
    let core = Core::open_temp("tags").unwrap();
    let t = tags::add_tag(&core.db, "db".into(), "#ff0000".into())
        .await
        .unwrap();
    assert!(
        tags::add_tag(&core.db, "db".into(), "#00ff00".into())
            .await
            .is_err(),
        "names are unique"
    );
    let t2 = tags::update_tag(
        &core.db,
        ops_ssh::tags::Tag {
            color: "#00ff00".into(),
            ..t.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(t2.color, "#00ff00");
    assert_eq!(tags::list_tags(&core.db).await.unwrap().len(), 1);
    tags::delete_tag(&core.db, t.id).await.unwrap();
    assert!(tags::list_tags(&core.db).await.unwrap().is_empty());
}

#[tokio::test]
async fn csv_roundtrip_skip_and_overwrite() {
    let core = Core::open_temp("csv").unwrap();
    let csv = "name,ip,port,auth_type,os,tags,group_id,remark\nweb,10.1.1.1,22,password,linux,,,first\nbad,,22,password,linux,,,x\ndb,10.1.1.2,2222,key,linux,,,second\n";
    let r = import::import_hosts_csv(&core.db, csv.into(), "incremental".into()).unwrap();
    assert_eq!((r.total, r.imported, r.skipped), (3, 2, 1), "{r:?}");
    assert!(
        r.errors.iter().any(|e| e.contains("line 3")),
        "{:?}",
        r.errors
    );

    let again = import::import_hosts_csv(&core.db, csv.into(), "incremental".into()).unwrap();
    assert_eq!(
        (again.imported, again.skipped),
        (0, 3),
        "incremental mode skips known IPs: {again:?}"
    );
    let over = import::import_hosts_csv(&core.db, csv.into(), "overwrite".into()).unwrap();
    assert_eq!(over.imported, 2, "{over:?}");
    assert_eq!(
        hosts::list_hosts(&core.db).await.unwrap().len(),
        2,
        "overwrite replaces, no duplicates"
    );

    let exported = import::export_hosts_csv(&core.db).unwrap();
    assert!(exported.starts_with("name,ip,port,auth_type,os,tags,group_id,remark"));
    assert!(exported.contains("10.1.1.2"));
}

#[tokio::test]
async fn jump_chain_is_resolved_and_cascades() {
    let core = Core::open_temp("jump").unwrap();
    let bastion = hosts::add_host(&core.db, new_host("bastion", "10.9.9.1"))
        .await
        .unwrap();
    let mut inner = new_host("inner", "10.9.9.2");
    inner.jump_chain = Some(serde_json::to_string(std::slice::from_ref(&bastion.id)).unwrap());
    let inner = hosts::add_host(&core.db, inner).await.unwrap();

    let chain = proxyjump::resolve_jump_chain(&core.db, inner.id.clone()).unwrap();
    assert_eq!(chain, vec![bastion.id.clone()]);
    let topo = proxyjump::get_jump_topology(&core.db).unwrap();
    let _ = topo;

    let registry = registry();
    let downstream =
        proxyjump::cascade_disconnect(&registry, &core.db, bastion.id.clone()).unwrap();
    // cascade_disconnect reports only downstream hosts that actually had a live pooled connection.
    assert!(
        downstream.is_empty(),
        "nothing was connected: {downstream:?}"
    );
}

#[tokio::test]
async fn check_host_status_reports_reachability() {
    let core = Core::open_temp("status").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port() as i32;
    let mut up = new_host("up", "127.0.0.1");
    up.port = Some(port);
    let up = hosts::add_host(&core.db, up).await.unwrap();
    let core2 = core.clone();
    let id = up.id.clone();
    let st = tokio::task::spawn_blocking(move || hosts::check_host_status(&core2.db, id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(st, "online");
    drop(listener);
    let core3 = core.clone();
    let id = up.id.clone();
    let st = tokio::task::spawn_blocking(move || hosts::check_host_status(&core3.db, id))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(st, "online");
}
