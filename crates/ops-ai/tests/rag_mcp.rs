use ops_ai::mcp::*;
use ops_ai::rag::*;

#[test]
fn rag_imports_markdown_and_ranks_matching_chunks_for_latin_and_cjk_queries() {
    let core = ops_core::Core::open_temp("rag").unwrap();
    let c = rag_create_collection(
        &core.db,
        "运维手册".into(),
        Some("ssh_host".into()),
        Some("h1".into()),
    )
    .unwrap();
    let pad = "Background paragraph that mentions nothing relevant to the question. ".repeat(8);
    let doc = format!("# Nginx\n\nReload nginx with `systemctl reload nginx` after editing the config.\n{pad}\n# 磁盘清理\n\n使用 `du -sh /var/*` 查看磁盘占用，然后清理日志文件。\n{pad}\n# Network\n\nUse `ss -tlnp` to list listening sockets.\n");
    let chunks = rag_import_document(&core.db, c.id.clone(), doc).unwrap();
    assert!(chunks >= 3, "{chunks}");
    let cols = rag_list_collections(&core.db, Some("ssh_host".into()), Some("h1".into())).unwrap();
    assert_eq!((cols.len(), cols[0].document_count), (1, 1));

    let hit = rag_search(
        &core.db,
        "reload nginx".into(),
        Some(vec![c.id.clone()]),
        Some(3),
    )
    .unwrap();
    assert!(
        hit[0].content.contains("systemctl reload nginx"),
        "{:?}",
        hit.iter().map(|h| &h.content).collect::<Vec<_>>()
    );
    let cjk = rag_search(
        &core.db,
        "磁盘占用 清理".into(),
        Some(vec![c.id.clone()]),
        Some(3),
    )
    .unwrap();
    assert!(
        cjk[0].content.contains("du -sh"),
        "{:?}",
        cjk.iter().map(|h| &h.content).collect::<Vec<_>>()
    );
    assert!(
        rag_search(&core.db, "zzzzqqq".into(), Some(vec![c.id.clone()]), None)
            .unwrap()
            .is_empty()
    );

    rag_delete_collection(&core.db, c.id.clone()).unwrap();
    assert!(rag_search(&core.db, "nginx".into(), None, None)
        .unwrap()
        .is_empty());
}

#[test]
fn mcp_server_configs_are_stored_and_removed() {
    let core = ops_core::Core::open_temp("mcp").unwrap();
    mcp_add_server(
        &core.db,
        McpServerConfig {
            id: "s1".into(),
            name: "fs".into(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: Some(vec!["-y".into(), "server".into()]),
            env: Default::default(),
            url: None,
            enabled: true,
        },
    )
    .unwrap();
    let list = mcp_list_servers(&core.db).unwrap();
    assert_eq!(
        (list.len(), list[0].args.clone()),
        (1, Some(vec!["-y".to_string(), "server".to_string()]))
    );
    mcp_remove_server(&core.db, "s1".into()).unwrap();
    assert!(mcp_list_servers(&core.db).unwrap().is_empty());
}
