//! Repo sync page against a local git repository.

mod common;

use std::process::Command;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use opsbatch_app::library::store::library_store;
use opsbatch_app::repo_sync::page::RepoSyncPage;

use common::{serial, wait_until};

fn git(dir: &std::path::Path, args: &[&str]) {
    let o = Command::new("git").current_dir(dir).args(["-c", "user.email=t@t", "-c", "user.name=t"]).args(args).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

#[gpui_kit::test]
fn add_sync_toggle_and_delete_a_repository_from_the_page(cx: &mut TestAppContext) {
    let _s = serial();
    let origin = std::env::temp_dir().join(format!("opsbatch-ui-repo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(origin.join("commands")).unwrap();
    git(&origin, &["init", "-b", "main"]);
    std::fs::write(origin.join("library_cn.json"), "{}").unwrap();
    std::fs::write(origin.join("commands/disk_cn.yaml"), "name: 磁盘\ncommand: df -h\ncategory: 磁盘\nrisk: low\ndescription: d\nplatform:\n  - linux\n").unwrap();
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-m", "init"]);

    let fx = common::setup(cx, |core| {
        // Drop the built-in repository so the page starts empty.
        ops_core::rt::block_on(ops_library::github::delete_repo(&core.db, "opsbatch-default-library-repo".into())).unwrap();
    });
    let (handle, page) = cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(RepoSyncPage::new)).unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    });
    let _ = &fx;
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("repo-empty").is_some() && w.try_find("repo-add-first").is_some());
    })
    .unwrap();

    // Validation, then add.
    let dlg = cx.update_window(handle.into(), |_, w, cx| page.update(cx, |p, cx| p.open_add(w, cx))).unwrap();
    cx.update_window(handle.into(), |_, w, cx| dlg.update(cx, |d, cx| d.submit(w, cx))).unwrap();
    cx.update(|cx| assert!(dlg.read(cx).url_missing()));
    cx.update_window(handle.into(), |_, w, cx| {
        dlg.update(cx, |d, cx| {
            d.url.update(cx, |s, cx| s.set_value(origin.to_string_lossy().to_string(), w, cx));
            d.submit(w, cx);
        });
    })
    .unwrap();
    wait_until(cx, "repo listed", |cx| cx.update(|cx| page.read(cx).repos().len() == 1));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("repo-card", 0usize)).is_some());
        w.click(("repo-sync", 0usize), cx);
    })
    .unwrap();
    wait_until(cx, "sync result", |cx| cx.update(|cx| page.read(cx).result().is_some() && !page.read(cx).is_pulling()));
    cx.update(|cx| {
        let r = page.read(cx).result().unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert!(!r.added.is_empty());
    });
    wait_until(cx, "last sync time shown", |cx| cx.update(|cx| page.read(cx).repos()[0].last_pulled_at.is_some()));
    // The libraries reload with the imported command.
    let store = cx.update(|cx| library_store(cx));
    wait_until(cx, "library refreshed", |cx| cx.update(|cx| store.read(cx).commands.iter().any(|c| c.name == "磁盘")));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("repo-result").is_some());
        w.click("repo-result-close", cx);
    })
    .unwrap();
    cx.update(|cx| assert!(page.read(cx).result().is_none()));

    // Pause: sync button disabled. Delete removes repo and its commands.
    cx.update_window(handle.into(), |_, w, cx| page.update(cx, |p, cx| {
        let id = p.repos()[0].id.clone();
        p.set_enabled(id, false, w, cx);
    }))
    .unwrap();
    wait_until(cx, "paused", |cx| cx.update(|cx| !page.read(cx).repos()[0].enabled));
    cx.update_window(handle.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find(("repo-sync", 0usize)).disabled(), Some(true));
        w.click(("repo-delete", 0usize), cx);
    })
    .unwrap();
    wait_until(cx, "repo deleted", |cx| cx.update(|cx| page.read(cx).repos().is_empty()));
    wait_until(cx, "library cleaned", |cx| cx.update(|cx| !store.read(cx).commands.iter().any(|c| c.name == "磁盘")));
    let _ = std::fs::remove_dir_all(&origin);
}
