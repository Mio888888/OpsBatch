//! Repo sync against a local git repository (no network): clone, import, re-pull, startup update, delete.

use std::path::Path;
use std::process::Command;

use ops_library::github::*;

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .output()
        .expect("git available");
    assert!(
        st.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&st.stderr)
    );
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn fixture_repo() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("opsbatch-lib-repo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-b", "main"]);
    write(&dir, "library_cn.json", "{}");
    write(&dir, "commands/disk_cn.yaml", "name: 磁盘使用\ncommand: df -h\ncategory: 磁盘\nrisk: low\ndescription: 查看磁盘\nplatform:\n  - linux\n");
    write(&dir, "docker-commands/ps_cn.yaml", "name: 容器列表\ncommand: docker ps\ncategory: 容器\nrisk: low\ndescription: docker ps\nplatform:\n  - linux\n");
    write(&dir, "scripts/backup_cn.sh", "#!/bin/sh\necho backup\n");
    write(
        &dir,
        "scripts/backup_cn.meta.json",
        r#"{"name":"备份","language":"shell","category":"维护","risk":"medium","description":"备份脚本","tags":["backup"],"platform":["linux"]}"#,
    );
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "init"]);
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn adding_pulling_repulling_and_deleting_a_repository() {
    let origin = fixture_repo();
    let core = ops_core::Core::open_temp("reposync").unwrap();
    // A fresh database ships with the default library repository; it must not take part in this test.
    assert_eq!(
        list_repos(&core.db).await.unwrap().len(),
        1,
        "built-in default repository present"
    );
    delete_repo(&core.db, "opsbatch-default-library-repo".into())
        .await
        .unwrap();
    let id = add_repo(
        &core.db,
        origin.to_string_lossy().to_string(),
        "main".into(),
        None,
        true,
    )
    .await
    .unwrap();
    let repos = list_repos(&core.db).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert!(
        !repos[0].has_token
            && repos[0].last_pulled_at.is_none()
            && repos[0].update_on_startup
            && repos[0].enabled
    );

    // First pull: everything is new.
    let r = pull_repo(
        core.data_dir().to_path_buf(),
        &core.db,
        id.clone(),
        Some("zh-CN".into()),
    )
    .await
    .unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(r.added.len() >= 3, "{:?}", r.added);
    let conn = core.db.pool.get().unwrap();
    let count = |sql: &str| {
        conn.query_row(sql, rusqlite::params![id], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM commands WHERE source_repo_id=?1"),
        2
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM scripts WHERE source_repo_id=?1"),
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM commands WHERE source_repo_id=?1 AND tags LIKE '%docker%'"),
        1,
        "docker commands are tagged"
    );
    assert!(list_repos(&core.db).await.unwrap()[0]
        .last_pulled_at
        .is_some());
    drop(conn);

    // Upstream change: a command disappears, a new one appears; second pull reports updates/removals.
    git(&origin, &["rm", "-q", "commands/disk_cn.yaml"]);
    write(&origin, "commands/mem_cn.yaml", "name: 内存\ncommand: free -m\ncategory: 内存\nrisk: low\ndescription: mem\nplatform:\n  - linux\n");
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-m", "change"]);
    let r = pull_repo(
        core.data_dir().to_path_buf(),
        &core.db,
        id.clone(),
        Some("zh-CN".into()),
    )
    .await
    .unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!r.updated.is_empty());
    let conn = core.db.pool.get().unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM commands WHERE source_repo_id=?1 ORDER BY name")
        .unwrap()
        .query_map(rusqlite::params![id], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(
        names.contains(&"内存".to_string()) && !names.contains(&"磁盘使用".to_string()),
        "{names:?}"
    );
    drop(conn);

    // Startup update pulls enabled + update_on_startup repos; disabled ones are skipped.
    let results = run_startup_repo_updates(&core).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].pulled, "{:?}", results[0]);
    toggle_repo(&core.db, id.clone(), false).await.unwrap();
    assert!(run_startup_repo_updates(&core).await.unwrap().is_empty());
    set_repo_update_on_startup(&core.db, id.clone(), false)
        .await
        .unwrap();
    toggle_repo(&core.db, id.clone(), true).await.unwrap();
    assert!(run_startup_repo_updates(&core).await.unwrap().is_empty());

    // Deleting a repo removes what it imported.
    delete_repo(&core.db, id.clone()).await.unwrap();
    let conn = core.db.pool.get().unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM commands WHERE source_repo_id=?1",
            rusqlite::params![id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(list_repos(&core.db).await.unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&origin);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_url_reports_an_error_instead_of_importing() {
    let core = ops_core::Core::open_temp("reposync-bad").unwrap();
    let id = add_repo(
        &core.db,
        "/definitely/not/a/repo".into(),
        "main".into(),
        None,
        false,
    )
    .await
    .unwrap();
    let err = pull_repo(core.data_dir().to_path_buf(), &core.db, id, None)
        .await
        .unwrap_err();
    assert!(err.to_lowercase().contains("clone failed"), "{err}");
}
