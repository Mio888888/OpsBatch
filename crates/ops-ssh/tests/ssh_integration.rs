mod support;

use std::time::Duration;

use ops_ssh::{execution, terminal};

fn drain_until(rx: &flume::Receiver<String>, needle: &str, secs: u64) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut all = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) {
            if chunk.is_empty() {
                break;
            }
            all.push_str(&chunk);
            if all.contains(needle) {
                break;
            }
        }
    }
    all
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_roundtrip_resize_and_close() {
    let Some(fx) = support::fixture("terminal").await else { return };
    let sid = terminal::terminal_connect(fx.ctx.clone(), fx.host_id.clone(), Some(100), Some(30), None)
        .await
        .expect("terminal_connect");
    let rx = fx.ctx.terminals.take(&sid).expect("terminal stream");

    terminal::terminal_write(&fx.ctx.terminals_mgr, &fx.ctx.registry, sid.clone(), "echo MARK$((20+22))X\n".into())
        .await
        .expect("write");
    let out = drain_until(&rx, "MARK42X", 15);
    assert!(out.contains("MARK42X"), "output was: {out:?}");

    terminal::terminal_resize(&fx.ctx.terminals_mgr, &fx.ctx.registry, sid.clone(), 120, 40).await.expect("resize");
    terminal::terminal_write(&fx.ctx.terminals_mgr, &fx.ctx.registry, sid.clone(), "stty size\n".into()).await.unwrap();
    let out = drain_until(&rx, "40 120", 10);
    assert!(out.contains("40 120"), "stty size after resize: {out:?}");

    terminal::terminal_disconnect(&fx.ctx.terminals_mgr, sid.clone(), Some(fx.host_id.clone())).await.expect("disconnect");
    let tail = drain_until(&rx, "\u{0}never", 5);
    let _ = tail;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_execute_records_history_and_events() {
    let Some(fx) = support::fixture("exec").await else { return };
    let task_id = "task-e2e-1".to_string();
    let rx = fx.ctx.events.subscribe(format!("exec:{task_id}:*"));
    let r = execution::execute_command(
        fx.ctx.clone(),
        &fx.ctx.db,
        task_id.clone(),
        vec![fx.host_id.clone()],
        "echo out-line; echo err-line 1>&2; exit 3".into(),
        4,
        30,
        None,
    );
    assert!(r.is_ok(), "{r:?}");

    // Wait for the done event.
    let mut saw_output = String::new();
    let mut done = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) {
            if ev.topic.ends_with(":output") {
                saw_output.push_str(&ev.payload.to_string());
            }
            if ev.topic.ends_with(":done") {
                done = Some(ev.payload.clone());
                break;
            }
        }
    }
    assert!(done.is_some(), "no done event; output so far: {saw_output}");
    assert!(saw_output.contains("out-line"), "{saw_output}");

    let history = execution::list_execution_history(&fx.ctx.db).await.unwrap();
    let h = history.iter().find(|h| h.id == task_id).expect("history row");
    assert_eq!(h.host_count, 1);
    let details = execution::get_execution_detail(&fx.ctx.db, task_id).await.unwrap();
    assert_eq!(details.len(), 1);
    assert!(details[0].output.contains("out-line"));
}

use ops_ssh::{forward, sftp, transfer};

/// Run a blocking (sync) service call off the async threads, like Tauri's command thread pool did.
macro_rules! blk {
    ($($v:ident),* => $body:expr) => {{
        $(let $v = $v.clone();)*
        tokio::task::spawn_blocking(move || $body).await.expect("join")
    }};
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sftp_full_lifecycle() {
    let Some(fx) = support::fixture("sftp").await else { return };
    let ctx = fx.ctx.clone();
    let host = fx.host_id.clone();
    let root = fx.work.to_string_lossy().to_string();

    sftp::sftp_open(ctx.clone(), host.clone()).await.expect("sftp_open");
    let home = blk!(ctx, host => sftp::sftp_home_dir(&ctx.sftp, host)).expect("home");
    assert!(!home.is_empty());

    // mkdir + write + read + list + stat + exists
    let dir = format!("{root}/dir1");
    blk!(ctx, host, dir => sftp::sftp_mkdir(&ctx.db, &ctx.sftp, &ctx.registry, host, dir)).expect("mkdir");
    let file = format!("{dir}/hello.txt");
    let content = "你好 sftp\nline2\n".to_string();
    blk!(ctx, host, file, content => sftp::sftp_write_file(&ctx.db, &ctx.sftp, &ctx.registry, host, file, content)).expect("write");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), content);
    let read = sftp::sftp_read_file(ctx.clone(), host.clone(), file.clone(), None).await.expect("read");
    assert!(!read.is_empty(), "read returned empty");
    let list = sftp::sftp_list_dir(ctx.clone(), host.clone(), dir.clone()).await.expect("list");
    assert!(list.iter().any(|e| e.name == "hello.txt" && !e.is_dir), "{list:?}");
    let st = blk!(ctx, host, file => sftp::sftp_stat(&ctx.sftp, host, file)).expect("stat");
    assert_eq!(st.size, content.len() as u64);
    assert!(blk!(ctx, host, file => sftp::sftp_exists(&ctx.sftp, host, file)).unwrap());

    // rename
    let renamed = format!("{dir}/renamed.txt");
    blk!(ctx, host, file, renamed => sftp::sftp_rename(&ctx.sftp, host, file, renamed)).expect("rename");
    assert!(std::path::Path::new(&renamed).exists() && !std::path::Path::new(&file).exists());

    // upload / download with progress events
    let local_src = fx.work.join("upload-src.bin");
    let payload: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&local_src, &payload).unwrap();
    let rx = ctx.events.subscribe("sftp-transfer:*");
    let remote_dst = format!("{dir}/uploaded.bin");
    sftp::sftp_upload(ctx.clone(), host.clone(), local_src.to_string_lossy().into(), remote_dst.clone(), "t-up".into())
        .await
        .expect("upload");
    assert_eq!(std::fs::read(&remote_dst).unwrap(), payload);
    let local_dst = fx.work.join("downloaded.bin");
    sftp::sftp_download(ctx.clone(), host.clone(), remote_dst.clone(), local_dst.to_string_lossy().into(), "t-down".into())
        .await
        .expect("download");
    assert_eq!(std::fs::read(&local_dst).unwrap(), payload);
    let topics: Vec<String> = rx.try_iter().map(|e| e.topic.clone()).collect();
    assert!(topics.iter().any(|t| t == "sftp-transfer:t-up:done"), "{topics:?}");
    assert!(topics.iter().any(|t| t == "sftp-transfer:t-down:done"), "{topics:?}");

    // archive extraction (remote `tar`) + tree
    let tar_dir = fx.work.join("tarsrc");
    std::fs::create_dir_all(tar_dir.join("inner")).unwrap();
    std::fs::write(tar_dir.join("inner/a.txt"), "A").unwrap();
    let archive = fx.work.join("pack.tar.gz");
    assert!(std::process::Command::new("tar").arg("czf").arg(&archive).arg("-C").arg(&tar_dir).arg(".").status().unwrap().success());
    let out_dir = format!("{root}/extracted");
    blk!(ctx, host, out_dir => sftp::sftp_mkdir(&ctx.db, &ctx.sftp, &ctx.registry, host, out_dir)).unwrap();
    let archive_s: String = archive.to_string_lossy().into();
    blk!(ctx, host, archive_s, out_dir => sftp::sftp_extract_archive(&ctx.db, &ctx.registry, host, archive_s, out_dir)).expect("extract");
    assert_eq!(std::fs::read_to_string(format!("{out_dir}/inner/a.txt")).unwrap(), "A");
    sftp::sftp_read_file_tree(ctx.clone(), host.clone(), root.clone(), Some(3)).await.expect("tree");

    // remove
    blk!(ctx, host, renamed => sftp::sftp_remove(&ctx.sftp, host, renamed)).expect("remove");
    blk!(ctx, host, remote_dst => sftp::sftp_remove(&ctx.sftp, host, remote_dst)).expect("remove2");
    blk!(ctx, host, dir => sftp::sftp_rmdir(&ctx.sftp, host, dir)).expect("rmdir");
    assert!(!std::path::Path::new(&dir).exists());
    blk!(ctx, host => sftp::sftp_close(&ctx.sftp, host)).expect("close");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_port_forward_echoes_through_ssh() {
    let Some(fx) = support::fixture("forward").await else { return };
    let ctx = fx.ctx.clone();
    let host = fx.host_id.clone();
    // Echo server the "remote side" will reach.
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_addr = echo.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = echo.accept().await else { break };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 1024];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let local_port = probe.local_addr().unwrap().port();
    drop(probe);

    let cfg = forward::ForwardConfig {
        forward_type: forward::ForwardType::Local,
        local_addr: format!("127.0.0.1:{local_port}"),
        remote_addr: echo_addr.to_string(),
        label: Some("echo".into()),
    };
    let entry = blk!(ctx, host, cfg => forward::forward_add(ctx.clone(), &ctx.db, &ctx.registry, &ctx.forwards, host, cfg)).expect("forward_add");

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // The listener comes up asynchronously and the first SSH channel can race it: allow a few attempts.
    let mut buf = [0u8; 64];
    let mut n = 0;
    for attempt in 0..5 {
        let mut c = match tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect(("127.0.0.1", local_port))).await {
            Ok(Ok(c)) => c,
            _ => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if c.write_all(b"ping-through-ssh").await.is_err() {
            continue;
        }
        match tokio::time::timeout(Duration::from_secs(10), c.read(&mut buf)).await {
            Ok(Ok(len)) if len > 0 => {
                n = len;
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(200 * (attempt + 1))).await,
        }
    }
    assert_eq!(&buf[..n], b"ping-through-ssh");

    let listed = blk!(ctx, host => forward::forward_list(&ctx.forwards, host)).unwrap();
    assert_eq!(listed.len(), 1);
    let fid = entry.id.clone();
    blk!(ctx, host, fid => forward::forward_stop(&ctx.forwards, host, fid)).expect("stop");
    blk!(ctx, host, fid => forward::forward_remove(&ctx.forwards, host, fid)).expect("remove");
    assert!(blk!(ctx, host => forward::forward_list(&ctx.forwards, host)).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_transfer_uploads_file_and_reports_done() {
    let Some(fx) = support::fixture("transfer").await else { return };
    let ctx = fx.ctx.clone();
    let src = fx.work.join("payload.txt");
    std::fs::write(&src, "batch-payload").unwrap();
    let remote_dir = fx.work.join("remote");
    std::fs::create_dir_all(&remote_dir).unwrap();
    let rx = ctx.events.subscribe("transfer:*");
    let req = transfer::TransferRequest {
        task_id: Some("xfer-1".into()),
        direction: "upload".into(),
        host_ids: vec![fx.host_id.clone()],
        local_path: src.to_string_lossy().into(),
        remote_path: format!("{}/payload.txt", remote_dir.display()),
        remote_paths: None,
        timeout: Some(30),
    };
    blk!(ctx => transfer::file_transfer(ctx.clone(), &ctx.db, req)).expect("file_transfer");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut done = None;
    let mut topics = Vec::new();
    while std::time::Instant::now() < deadline && done.is_none() {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) {
            topics.push(ev.topic.clone());
            if ev.topic.ends_with(":done") {
                done = Some(ev.payload.clone());
            }
        }
    }
    assert!(done.is_some(), "no transfer done event: {topics:?}");
    assert_eq!(std::fs::read_to_string(remote_dir.join("payload.txt")).unwrap(), "batch-payload");
}

fn wait_done(rx: &flume::Receiver<std::sync::Arc<ops_core::events::AppEvent>>, secs: u64) -> (String, Option<serde_json::Value>) {
    let mut out = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(200)) {
            if ev.topic.ends_with(":output") {
                out.push_str(&ev.payload.to_string());
            }
            if ev.topic.ends_with(":done") {
                return (out, Some(ev.payload.clone()));
            }
        }
    }
    (out, None)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exec_through_jump_host_chain() {
    let Some(fx) = support::fixture("jump-exec").await else { return };
    let ctx = fx.ctx.clone();
    // Host B reaches the same sshd *through* host A (direct-tcpip over A's connection).
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let b = ops_ssh::hosts::add_host(
        &ctx.db,
        ops_ssh::hosts::NewHost {
            name: "via-jump".into(),
            ip: "127.0.0.1".into(),
            port: Some(fx.sshd.port as i32),
            auth_type: Some("key".into()),
            username: Some(user),
            password: None,
            private_key: Some(fx.sshd.private_key.clone()),
            os: Some("linux".into()),
            tags: None,
            group_id: None,
            remark: None,
            jump_chain: Some(serde_json::to_string(&[fx.host_id.clone()]).unwrap()),
            rdp_settings: None,
            proxy_settings: None,
        },
    )
    .await
    .unwrap();
    let rx = ctx.events.subscribe("exec:jump-1:*");
    execution::execute_command(ctx.clone(), &ctx.db, "jump-1".into(), vec![b.id.clone()], "echo via-jump-ok".into(), 2, 30, None)
        .expect("execute");
    let (out, done) = wait_done(&rx, 25);
    assert!(done.is_some(), "no done: {out}");
    assert!(out.contains("via-jump-ok"), "{out}");
}

/// The `timeout` argument bounds *connecting* (as in the Tauri build): an unreachable host must fail fast with a
/// recorded result instead of hanging the batch. A long-running command is NOT cut off by it (documented parity).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exec_connect_timeout_fails_fast_and_is_recorded() {
    let Some(fx) = support::fixture("exec-timeout").await else { return };
    let ctx = fx.ctx.clone();
    // 192.0.2.0/24 is TEST-NET-1: never routable, so the TCP connect cannot succeed.
    let dead = ops_ssh::hosts::add_host(
        &ctx.db,
        ops_ssh::hosts::NewHost {
            name: "dead".into(),
            ip: "192.0.2.1".into(),
            port: Some(22),
            auth_type: Some("password".into()),
            username: Some("root".into()),
            password: Some("x".into()),
            private_key: None,
            os: Some("linux".into()),
            tags: None,
            group_id: None,
            remark: None,
            jump_chain: None,
            rdp_settings: None,
            proxy_settings: None,
        },
    )
    .await
    .unwrap();
    let rx = ctx.events.subscribe("exec:to-1:*");
    let started = std::time::Instant::now();
    execution::execute_command(ctx.clone(), &ctx.db, "to-1".into(), vec![dead.id.clone()], "true".into(), 1, 2, None).unwrap();
    let (_out, done) = wait_done(&rx, 30);
    assert!(done.is_some(), "no done event for an unreachable host");
    assert!(started.elapsed() < Duration::from_secs(20), "connect timeout did not bound the wait: {:?}", started.elapsed());
    let d = execution::get_execution_detail(&ctx.db, "to-1".into()).await.unwrap();
    assert_eq!(d.len(), 1);
    assert_ne!(d[0].status, "success", "{:?}", d[0].status);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_pty_terminal_and_background_process() {
    let Some(fx) = support::fixture("local-pty").await else { return };
    let ctx = fx.ctx.clone();
    let sid = blk!(ctx => terminal::terminal_connect_local(ctx.clone(), &ctx.terminals_mgr, Some(80), Some(24))).unwrap_or_else(|e| panic!("local connect: {e}"));
    let rx = ctx.terminals.take(&sid).expect("stream");
    terminal::terminal_write(&ctx.terminals_mgr, &ctx.registry, sid.clone(), "echo PTY$((6*7))Z\n".into()).await.unwrap();
    let out = drain_until(&rx, "PTY42Z", 15);
    assert!(out.contains("PTY42Z"), "{out:?}");
    terminal::terminal_disconnect(&ctx.terminals_mgr, sid, None).await.unwrap();

    let marker = fx.work.join("bg-marker.txt");
    let info = terminal::terminal_spawn_background_process(
        ctx.clone(),
        format!("echo background-ran > '{}'", marker.display()),
        None,
        None,
    )
    .await
    .expect("spawn bg");
    assert!(info.pid > 0);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "background-ran");
    assert!(std::path::Path::new(&info.log_file).starts_with(fx.ctx.data_dir()), "log under data dir: {}", info.log_file);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn monitor_snapshot_does_not_panic_on_non_linux() {
    let Some(fx) = support::fixture("monitor").await else { return };
    let r = ops_ssh::hosts::get_host_monitor_snapshot(fx.ctx.core.clone(), fx.ctx.registry.clone(), fx.host_id.clone()).await;
    // Linux/procfs hosts yield a snapshot; others may return partial data or a clear error — never a panic/hang.
    match r {
        Ok(s) => eprintln!("MONITOR ok: os={:?} procs={} fs={}", s.os, s.processes.len(), s.filesystems.len()),
        Err(e) => eprintln!("MONITOR err (acceptable off-Linux): {e}"),
    }
}
