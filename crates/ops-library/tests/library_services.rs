use ops_library::library::*;
use ops_library::quick_actions::*;

fn core() -> ops_core::Core {
    ops_core::Core::open_temp("library").unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_crud_star_and_builtin_protection() {
    let core = core();
    let id = add_command(
        &core.db,
        NewCommand {
            name: "disk".into(),
            command: "df -h".into(),
            category: Some("system".into()),
            tags: None,
            risk: None,
            description: None,
            platform: None,
            parameters: None,
            url: None,
        },
    )
    .await
    .unwrap();
    let mut all = list_commands(&core.db).await.unwrap();
    let mine = all.iter().find(|c| c.id == id).unwrap().clone();
    assert_eq!(
        (
            mine.risk.as_str(),
            mine.platform.as_str(),
            mine.tags.as_str(),
            mine.starred
        ),
        ("low", "linux", "[]", false)
    );

    toggle_star_command(&core.db, id.clone()).await.unwrap();
    all = list_commands(&core.db).await.unwrap();
    assert!(all.iter().find(|c| c.id == id).unwrap().starred);

    let mut edited = mine.clone();
    edited.command = "df -hT".into();
    edited.starred = true;
    update_command(&core.db, edited).await.unwrap();
    assert_eq!(
        list_commands(&core.db)
            .await
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .command,
        "df -hT"
    );

    // Built-in rows survive delete.
    let conn = core.db.pool.get().unwrap();
    conn.execute(
        "INSERT INTO commands (id,name,command,is_builtin) VALUES ('b1','builtin','true',1)",
        [],
    )
    .unwrap();
    drop(conn);
    delete_command(&core.db, "b1".into()).await.unwrap();
    delete_command(&core.db, id.clone()).await.unwrap();
    let all = list_commands(&core.db).await.unwrap();
    assert!(all.iter().any(|c| c.id == "b1") && !all.iter().any(|c| c.id == id));
    // built-ins sort first
    assert_eq!(all.first().map(|c| c.id.as_str()), Some("b1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn scripts_and_versions() {
    let core = core();
    let id = add_script(
        &core.db,
        NewScript {
            name: "backup".into(),
            language: None,
            category: None,
            tags: None,
            risk: None,
            description: None,
            content: Some("echo 1".into()),
            parameters: None,
            url: None,
            platform: None,
        },
    )
    .await
    .unwrap();
    let s = list_scripts(&core.db)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap();
    assert_eq!(
        (s.language.as_str(), s.content.as_str()),
        ("shell", "echo 1")
    );
    save_script_version(&core.db, id.clone(), "echo 1".into(), "v1".into())
        .await
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    save_script_version(&core.db, id.clone(), "echo 2".into(), "v2".into())
        .await
        .unwrap();
    let versions = list_script_versions(&core.db, id.clone()).await.unwrap();
    assert_eq!(
        versions
            .iter()
            .map(|v| v.label.as_str())
            .collect::<Vec<_>>(),
        ["v2", "v1"],
        "newest first"
    );
    toggle_star_script(&core.db, id.clone()).await.unwrap();
    assert!(list_scripts(&core.db)
        .await
        .unwrap()
        .iter()
        .any(|s| s.id == id && s.starred));
    delete_script(&core.db, id.clone()).await.unwrap();
    assert!(list_scripts(&core.db)
        .await
        .unwrap()
        .iter()
        .all(|s| s.id != id));
}

#[tokio::test(flavor = "multi_thread")]
async fn quick_actions_reorder_star_and_update() {
    let core = core();
    let mut ids = Vec::new();
    for n in ["a", "b", "c"] {
        ids.push(
            add_quick_action(
                &core.db,
                NewQuickAction {
                    name: n.into(),
                    command: format!("echo {n}"),
                    category: None,
                    parameters: None,
                    sort_order: None,
                    description: None,
                    tags: None,
                    language: None,
                },
            )
            .await
            .unwrap(),
        );
    }
    reorder_quick_actions(
        &core.db,
        vec![ids[2].clone(), ids[0].clone(), ids[1].clone()],
    )
    .await
    .unwrap();
    let names: Vec<_> = list_quick_actions(&core.db)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["c", "a", "b"]);
    toggle_star_quick_action(&core.db, ids[0].clone())
        .await
        .unwrap();
    let mut a = list_quick_actions(&core.db)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.id == ids[0])
        .unwrap();
    assert_eq!(a.starred, 1);
    a.command = "echo changed".into();
    update_quick_action(&core.db, a).await.unwrap();
    assert_eq!(
        list_quick_actions(&core.db)
            .await
            .unwrap()
            .into_iter()
            .find(|a| a.id == ids[0])
            .unwrap()
            .command,
        "echo changed"
    );
    delete_quick_action(&core.db, ids[1].clone()).await.unwrap();
    assert_eq!(list_quick_actions(&core.db).await.unwrap().len(), 2);
}
