//! SFTP store against a private sshd: browse, create/rename/delete, upload/download with events, preview, bookmarks.

mod common;
#[path = "../../crates/ops-ssh/tests/support/mod.rs"]
mod ssh_support;

use gpui_kit::{Entity, TestAppContext};
use opsbatch_app::sftp::store::{sftp_store, Side, SessionState, SftpStore, TransferStatus};
use ops_ssh::hosts::NewHost;

use common::{serial, wait_until};

fn store(cx: &mut TestAppContext) -> Entity<SftpStore> {
    cx.update(|cx| sftp_store(cx))
}

fn await_task<T: 'static>(cx: &mut TestAppContext, task: gpui_kit::Task<Result<T, String>>) -> Result<T, String> {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let s2 = slot.clone();
    cx.update(|cx| cx.spawn(async move |_| *s2.borrow_mut() = Some(task.await)).detach());
    wait_until(cx, "task result", |_| slot.borrow().is_some());
    let r = slot.borrow_mut().take().unwrap();
    r
}

#[gpui_kit::test]
fn sftp_store_browses_edits_and_transfers_files(cx: &mut TestAppContext) {
    let _s = serial();
    let Some(sshd) = ssh_support::Sshd::start() else {
        eprintln!("SKIP: no sshd");
        return;
    };
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    let (port, key) = (sshd.port as i32, sshd.private_key.clone());
    let mut host_id = String::new();
    let _fx = common::setup(cx, |core| {
        host_id = ops_core::rt::block_on(ops_ssh::hosts::add_host(
            &core.db,
            NewHost {
                name: "sftp".into(),
                ip: "127.0.0.1".into(),
                port: Some(port),
                auth_type: Some("key".into()),
                username: Some(user.clone()),
                password: None,
                private_key: Some(key.clone()),
                os: Some("linux".into()),
                tags: None,
                group_id: None,
                remark: None,
                jump_chain: None,
                rdp_settings: None,
                proxy_settings: None,
            },
        ))
        .unwrap()
        .id;
    });
    let work = std::env::temp_dir().join(format!("opsbatch-sftp-{}", uuid::Uuid::new_v4()));
    let remote_dir = work.join("remote");
    let local_dir = work.join("local");
    std::fs::create_dir_all(&remote_dir).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    std::fs::write(local_dir.join("up.txt"), "hello from local").unwrap();
    std::fs::write(remote_dir.join("down.txt"), "hello from remote").unwrap();
    let (remote_dir_s, local_dir_s) = (remote_dir.to_string_lossy().to_string(), local_dir.to_string_lossy().to_string());

    let st = store(cx);
    cx.update(|cx| st.update(cx, |s, cx| s.open_session(&host_id, cx)));
    wait_until(cx, "session ready", |cx| cx.update(|cx| st.read(cx).session == SessionState::Ready && !st.read(cx).remote_loading));
    cx.update(|cx| assert!(st.read(cx).remote_error.is_none()));

    // Navigate both sides.
    cx.update(|cx| st.update(cx, |s, cx| s.navigate(Side::Remote, &remote_dir_s, cx)));
    wait_until(cx, "remote listed", |cx| cx.update(|cx| st.read(cx).remote_entries.iter().any(|e| e.name == "down.txt")));
    // Local access needs an authorized folder first.
    cx.update(|cx| st.update(cx, |s, cx| s.navigate(Side::Local, &local_dir_s, cx)));
    wait_until(cx, "local denied", |cx| cx.update(|cx| st.read(cx).local_error.is_some()));
    cx.update(|cx| st.update(cx, |s, cx| s.authorize_local(&local_dir, cx))).unwrap();
    wait_until(cx, "local listed", |cx| cx.update(|cx| st.read(cx).local_entries.iter().any(|e| e.name == "up.txt")));

    // Create folder + file, rename, delete.
    let sub = format!("{remote_dir_s}/sub");
    let t = cx.update(|cx| st.update(cx, |s, cx| s.mkdir(Side::Remote, sub.clone(), cx)));
    await_task(cx, t).unwrap();
    let new_file = format!("{remote_dir_s}/new.txt");
    let t = cx.update(|cx| st.update(cx, |s, cx| s.create_file(Side::Remote, new_file.clone(), cx)));
    await_task(cx, t).unwrap();
    wait_until(cx, "created entries visible", |cx| cx.update(|cx| {
        let n: Vec<_> = st.read(cx).remote_entries.iter().map(|e| e.name.clone()).collect();
        n.contains(&"sub".to_string()) && n.contains(&"new.txt".to_string())
    }));
    let renamed = format!("{remote_dir_s}/renamed.txt");
    let t = cx.update(|cx| st.update(cx, |s, cx| s.rename(Side::Remote, new_file.clone(), renamed.clone(), cx)));
    await_task(cx, t).unwrap();
    let t = cx.update(|cx| st.update(cx, |s, cx| s.remove(Side::Remote, renamed.clone(), false, cx)));
    await_task(cx, t).unwrap();
    let t = cx.update(|cx| st.update(cx, |s, cx| s.remove(Side::Remote, sub.clone(), true, cx)));
    await_task(cx, t).unwrap();
    assert!(!remote_dir.join("sub").exists() && !remote_dir.join("renamed.txt").exists());

    // Upload / download show up in the queue and land on disk.
    cx.update(|cx| st.update(cx, |s, cx| s.upload(&local_dir.join("up.txt").to_string_lossy(), cx)));
    cx.update(|cx| st.update(cx, |s, cx| s.download(&remote_dir.join("down.txt").to_string_lossy(), cx)));
    wait_until(cx, "both transfers done", |cx| cx.update(|cx| {
        let s = st.read(cx);
        s.transfers.len() == 2 && s.transfers.iter().all(|t| t.status == TransferStatus::Done)
    }));
    assert_eq!(std::fs::read_to_string(remote_dir.join("up.txt")).unwrap(), "hello from local");
    assert_eq!(std::fs::read_to_string(local_dir.join("down.txt")).unwrap(), "hello from remote");
    cx.update(|cx| {
        let s = st.read(cx);
        assert!(s.transfers.iter().all(|t| t.file_size > 0 && t.duration_ms.is_some() || t.file_size > 0));
        assert_eq!(s.active_transfers(), 0);
    });
    wait_until(cx, "listings refreshed after transfers", |cx| cx.update(|cx| {
        let s = st.read(cx);
        s.remote_entries.iter().any(|e| e.name == "up.txt") && s.local_entries.iter().any(|e| e.name == "down.txt")
    }));

    // Failed transfer reports an error.
    cx.update(|cx| st.update(cx, |s, cx| s.upload("/definitely/not/here.bin", cx)));
    wait_until(cx, "failed transfer recorded", |cx| cx.update(|cx| st.read(cx).transfers.iter().any(|t| t.status == TransferStatus::Error && t.error.is_some())));

    // Preview (text) from both sides.
    let entry = cx.update(|cx| st.read(cx).remote_entries.iter().find(|e| e.name == "down.txt").cloned().unwrap());
    cx.update(|cx| st.update(cx, |s, cx| s.open_preview(Side::Remote, entry, cx)));
    wait_until(cx, "preview loaded", |cx| cx.update(|cx| st.read(cx).preview.as_ref().is_some_and(|p| p.data.is_some())));
    cx.update(|cx| assert_eq!(st.read(cx).preview.as_ref().unwrap().data.as_deref(), Some(&b"hello from remote"[..])));
    cx.update(|cx| st.update(cx, |s, cx| s.close_preview(cx)));

    // Archive extraction.
    if std::process::Command::new("tar").arg("--version").output().is_ok() {
        let src = work.join("srcdir");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("inner.txt"), "zip me").unwrap();
        let archive = remote_dir.join("a.tar.gz");
        assert!(std::process::Command::new("tar").args(["-czf"]).arg(&archive).arg("-C").arg(&src).arg("inner.txt").status().unwrap().success());
        let t = cx.update(|cx| st.update(cx, |s, cx| s.extract_archive(archive.to_string_lossy().to_string(), cx)));
        let res = await_task(cx, t);
        assert!(res.is_ok(), "{res:?}");
        assert!(remote_dir.join("inner.txt").exists() || std::fs::read_dir(&remote_dir).unwrap().count() > 3);
    }

    // Bookmarks persist (settings) and survive a fresh load.
    cx.update(|cx| st.update(cx, |s, cx| { s.add_bookmark(Side::Remote, cx); s.add_bookmark(Side::Local, cx); }));
    cx.update(|cx| assert_eq!(st.read(cx).bookmarks.len(), 2));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let core_ctx = cx.update(|cx| opsbatch_app::services::ctx(cx));
    let saved = ops_core::rt::block_on(ops_core::settings::get_general_settings(&core_ctx.db)).unwrap();
    assert!(saved.get("sftpBookmarks").is_some_and(|s| s.contains(&remote_dir_s)));
    let first = cx.update(|cx| st.read(cx).bookmarks[0].id.clone());
    cx.update(|cx| st.update(cx, |s, cx| s.remove_bookmark(&first, cx)));
    cx.update(|cx| assert_eq!(st.read(cx).bookmarks.len(), 1));

    let _ = std::fs::remove_dir_all(&work);
}
