//! The vault unlock flow (port of `KeychainNoticeModal`): automatic unlock, asking for the startup password, wrong
//! passwords, migration failures and the shared pending request. A fake backend stands in for the system keychain.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use opsbatch_app::vault::{self, VaultBackend};
use opsbatch_app::vault_dialog::{MISSING_KEY, STALE_KEY};

use common::{serial, wait_until, Fixture};

#[derive(Clone)]
enum Mode {
    /// The system keychain holds a key that opens the vault.
    Keychain,
    /// No key in the keychain: only the typed password opens it.
    NeedsPassword(&'static str),
    /// The keychain key no longer opens the vault.
    Stale(&'static str),
    /// The keychain cannot be used at all (no Secret Service): only the typed password opens it.
    Unavailable(&'static str),
    /// The keychain refuses with an error a password does not fix, for the first `n` tries.
    Broken(usize),
}

struct Fake {
    mode: Mode,
    unlocked: AtomicBool,
    attempts: Mutex<Vec<Option<String>>>,
    migrations: AtomicUsize,
    migrate_error: Mutex<Option<String>>,
}

impl Fake {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            unlocked: AtomicBool::new(false),
            attempts: Mutex::new(Vec::new()),
            migrations: AtomicUsize::new(0),
            migrate_error: Mutex::new(None),
        })
    }
}

impl VaultBackend for Fake {
    fn is_unlocked(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }
    fn unlock(&self, key: Option<String>) -> Result<(), String> {
        self.attempts.lock().unwrap().push(key.clone());
        let open = match (&self.mode, key.as_deref()) {
            (Mode::Keychain, _) => true,
            (Mode::Broken(n), _) => {
                if self.attempts.lock().unwrap().len() <= *n {
                    return Err("keyring 读取失败: 安全存储不可访问".into());
                }
                true
            }
            (Mode::Unavailable(_), None) => return Err("系统钥匙串不可用，请输入启动密钥解锁，本次不会保存到钥匙串。详情: no Secret Service".into()),
            (Mode::NeedsPassword(p) | Mode::Stale(p) | Mode::Unavailable(p), Some(k)) if k.trim() == *p => true,
            (Mode::NeedsPassword(_), None) => return Err("本地加密密钥不存在，请输入新的解锁密钥。".into()),
            (Mode::Stale(_), None) => return Err("系统钥匙串中的本地加密密钥无法解开现有 vault，请输入启动密钥。".into()),
            _ => return Err("启动密钥不正确".into()),
        };
        self.unlocked.store(open, Ordering::SeqCst);
        Ok(())
    }
    fn after_unlock(&self, _db: &ops_core::Database) -> Result<(), String> {
        self.migrations.fetch_add(1, Ordering::SeqCst);
        match self.migrate_error.lock().unwrap().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

fn win<R>(
    cx: &mut TestAppContext,
    fx: &Fixture,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(fx.handle.into(), |_, window, cx| f(window, cx))
        .unwrap()
}

fn setup(cx: &mut TestAppContext, backend: Arc<Fake>) -> Fixture {
    let fx = common::setup(cx, |_| {});
    let b: Arc<dyn VaultBackend> = backend;
    cx.update(|cx| vault::set_backend(b, cx));
    fx
}

fn request(cx: &mut TestAppContext, fx: &Fixture) -> futures::channel::oneshot::Receiver<bool> {
    win(cx, fx, vault::request_unlock)
}

fn dialog_shown(cx: &mut TestAppContext, fx: &Fixture) -> bool {
    win(cx, fx, |w, cx| {
        w.render_frame(cx);
        w.try_find("vault-dialog").is_some()
    })
}

#[gpui_kit::test]
fn a_key_in_the_system_keychain_unlocks_without_asking(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::Keychain);
    let fx = setup(cx, fake.clone());
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    cx.update(|cx| vault::on_unlocked(cx, move |_| flag.store(true, Ordering::SeqCst)));
    assert!(
        !ran.load(Ordering::SeqCst),
        "waiters do not run while the vault is locked"
    );
    let mut rx = request(cx, &fx);
    assert!(
        dialog_shown(cx, &fx),
        "the dialog opens while the unlock is under way"
    );
    wait_until(cx, "unlocked", |_| {
        fake.is_unlocked() && ran.load(Ordering::SeqCst)
    });
    assert_eq!(rx.try_recv().ok().flatten(), Some(true));
    assert_eq!(
        fake.migrations.load(Ordering::SeqCst),
        1,
        "plaintext secrets are migrated once after the unlock"
    );
    assert_eq!(
        *fake.attempts.lock().unwrap(),
        vec![None],
        "the first attempt carries no password"
    );
    cx.run_until_parked();
    assert!(!dialog_shown(cx, &fx), "the dialog closes by itself");
    // afterwards everything is immediate
    let mut again = request(cx, &fx);
    assert_eq!(again.try_recv().ok().flatten(), Some(true));
    let late = Arc::new(AtomicBool::new(false));
    let flag = late.clone();
    cx.update(|cx| vault::on_unlocked(cx, move |_| flag.store(true, Ordering::SeqCst)));
    assert!(late.load(Ordering::SeqCst));
}

#[gpui_kit::test]
fn without_a_keychain_key_the_dialog_asks_for_the_startup_password(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::NeedsPassword("s3cret"));
    let fx = setup(cx, fake.clone());
    let mut rx = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "password form", |cx| {
        dialog.read_with(cx, |d, _| d.needs_password() && !d.loading())
    });
    dialog.read_with(cx, |d, _| {
        assert_eq!(
            d.error(),
            ops_i18n::template(ops_i18n::Language::ZhCn, "keychainNotice.needMasterKey")
        )
    });
    assert!(!MISSING_KEY.is_empty());
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(
            w.try_find("vault-password-form").is_some()
                && w.try_find("vault-unlock").is_some()
                && w.try_find("vault-error").is_some()
        );
        assert!(w.try_find("vault-hints").is_none());
    });
    assert!(
        rx.try_recv().ok().flatten().is_none(),
        "nothing proceeds while locked"
    );

    // a wrong password is reported and the dialog stays
    let input = dialog.read_with(cx, |d, _| d.password_input().clone());
    win(cx, &fx, |w, cx| {
        input.update(cx, |s, cx| s.set_value("wrong".to_string(), w, cx));
        dialog.update(cx, |d, cx| d.submit(w, cx));
    });
    wait_until(cx, "wrong password answered", |cx| {
        dialog.read_with(cx, |d, _| !d.loading() && d.error() == "启动密钥不正确")
    });
    assert!(!fake.is_unlocked() && dialog_shown(cx, &fx));
    assert_eq!(fake.migrations.load(Ordering::SeqCst), 0);

    // the right one (Enter on the field) unlocks, migrates and resolves the request
    win(cx, &fx, |w, cx| {
        input.update(cx, |s, cx| s.set_value("  s3cret ".to_string(), w, cx));
        input.update(cx, |_, cx| {
            cx.emit(gpui_kit::component::input::InputEvent::PressEnter {
                secondary: false,
                shift: false,
            })
        });
    });
    wait_until(cx, "unlocked", |_| fake.is_unlocked());
    cx.run_until_parked();
    assert_eq!(rx.try_recv().ok().flatten(), Some(true));
    assert_eq!(fake.migrations.load(Ordering::SeqCst), 1);
    assert_eq!(
        *fake.attempts.lock().unwrap(),
        vec![
            None,
            Some("wrong".to_string()),
            Some("  s3cret ".to_string())
        ]
    );
    assert!(!dialog_shown(cx, &fx));
}

#[gpui_kit::test]
fn a_stale_keychain_key_gets_its_own_message(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::Stale("old-password"));
    let fx = setup(cx, fake.clone());
    let _rx = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "password form", |cx| {
        dialog.read_with(cx, |d, _| d.needs_password())
    });
    dialog.read_with(cx, |d, _| {
        assert_eq!(
            d.error(),
            ops_i18n::template(ops_i18n::Language::ZhCn, "keychainNotice.staleMasterKey")
        )
    });
    assert!(!STALE_KEY.is_empty());
}

#[gpui_kit::test]
fn an_unusable_keychain_still_offers_the_startup_password_with_the_reason(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::Unavailable("pw"));
    let fx = setup(cx, fake.clone());
    let mut rx = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "password form", |cx| {
        dialog.read_with(cx, |d, _| d.needs_password() && !d.loading())
    });
    dialog.read_with(cx, |d, _| {
        assert!(
            d.error().starts_with(ops_i18n::template(
                ops_i18n::Language::ZhCn,
                "keychainNotice.keychainUnavailable"
            )),
            "{}",
            d.error()
        );
        assert!(
            d.error().contains("no Secret Service"),
            "the reason is shown: {}",
            d.error()
        );
        assert!(!d.failed(), "a password helps here");
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("vault-password-form").is_some() && w.try_find("vault-retry").is_none());
    });
    // the password opens it
    let input = dialog.read_with(cx, |d, _| d.password_input().clone());
    win(cx, &fx, |w, cx| {
        input.update(cx, |s, cx| s.set_value("pw".to_string(), w, cx));
        dialog.update(cx, |d, cx| d.submit(w, cx));
    });
    wait_until(cx, "unlocked", |_| fake.is_unlocked());
    cx.run_until_parked();
    assert_eq!(rx.try_recv().ok().flatten(), Some(true));
}

#[gpui_kit::test]
fn an_error_a_password_cannot_fix_offers_a_retry_instead_of_spinning(cx: &mut TestAppContext) {
    let _s = serial();
    // the keychain refuses twice (denied, then the prompt dismissed), then lets go
    let fake = Fake::new(Mode::Broken(2));
    let fx = setup(cx, fake.clone());
    let mut rx = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "first failure", |cx| {
        dialog.read_with(cx, |d, _| d.failed() && !d.loading())
    });
    dialog.read_with(cx, |d, _| {
        assert!(d.error().contains("安全存储不可访问"), "{}", d.error());
        assert!(!d.needs_password());
    });
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("vault-retry").is_some(), "a retry button");
        assert!(
            w.try_find("vault-password-form").is_none(),
            "no password field: it would not help"
        );
    });
    assert!(rx.try_recv().ok().flatten().is_none());
    // retry: still refused (second failure) ...
    win(cx, &fx, |w, cx| w.click("vault-retry", cx));
    wait_until(cx, "second failure", |cx| {
        dialog.read_with(cx, |d, _| d.failed() && !d.loading())
            && fake.attempts.lock().unwrap().len() == 2
    });
    // ... and the third try goes through
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        w.click("vault-retry", cx);
    });
    wait_until(cx, "unlocked", |_| fake.is_unlocked());
    cx.run_until_parked();
    assert_eq!(rx.try_recv().ok().flatten(), Some(true));
    assert_eq!(fake.attempts.lock().unwrap().len(), 3);
}

#[gpui_kit::test]
fn every_caller_shares_one_dialog_and_one_unlock(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::NeedsPassword("pw"));
    let fx = setup(cx, fake.clone());
    let mut first = request(cx, &fx);
    let mut second = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "password form", |cx| {
        dialog.read_with(cx, |d, _| d.needs_password() && !d.loading())
    });
    assert_eq!(
        fake.attempts.lock().unwrap().len(),
        1,
        "a second request does not start a second unlock"
    );
    let input = dialog.read_with(cx, |d, _| d.password_input().clone());
    win(cx, &fx, |w, cx| {
        input.update(cx, |s, cx| s.set_value("pw".to_string(), w, cx));
        dialog.update(cx, |d, cx| d.submit(w, cx));
    });
    wait_until(cx, "unlocked", |_| fake.is_unlocked());
    cx.run_until_parked();
    assert_eq!(
        (
            first.try_recv().ok().flatten(),
            second.try_recv().ok().flatten()
        ),
        (Some(true), Some(true))
    );
    assert_eq!(fake.migrations.load(Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn a_failed_migration_is_shown_and_keeps_the_dialog_open(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::Keychain);
    *fake.migrate_error.lock().unwrap() = Some("迁移失败: disk full".into());
    let fx = setup(cx, fake.clone());
    let mut rx = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "migration failed", |cx| {
        dialog.read_with(cx, |d, _| !d.loading() && d.error().contains("disk full"))
    });
    assert!(
        rx.try_recv().ok().flatten().is_none(),
        "the request stays unanswered"
    );
    assert!(dialog_shown(cx, &fx));
    assert!(
        !dialog.read_with(cx, |d, _| d.needs_password()),
        "this is not a password problem"
    );
}

#[gpui_kit::test]
fn the_ai_preferences_wait_for_the_vault(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;
    let _s = serial();
    let fake = Fake::new(Mode::Keychain);
    let fx = setup(cx, fake.clone());
    // a store created while the vault is locked reads nothing (it would see blank keys and could erase them)
    let store = cx.update(|cx| cx.new(opsbatch_app::ai::prefs_store::PrefsStore::new));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!store.read_with(cx, |s, _| s.loaded()));
    cx.update(|cx| store.update(cx, |s, cx| s.update(cx, |p| p.max_iterations = 5)));
    assert!(
        !store.read_with(cx, |s, _| s.saving()),
        "edits are ignored until it is loaded"
    );
    let _rx = request(cx, &fx);
    wait_until(cx, "loaded after the unlock", |cx| {
        store.read_with(cx, |s, _| s.loaded())
    });
    assert_eq!(store.read_with(cx, |s, _| s.prefs().max_iterations), 20);
}

#[gpui_kit::test]
fn the_keyboard_unlocks_and_the_button_works(cx: &mut TestAppContext) {
    let _s = serial();
    let fake = Fake::new(Mode::NeedsPassword("pw"));
    let fx = setup(cx, fake.clone());
    let _rx = request(cx, &fx);
    let dialog = loop {
        if let Some(d) = cx.update(|cx| vault::dialog(cx)) {
            break d;
        }
        cx.run_until_parked();
    };
    wait_until(cx, "password form", |cx| {
        dialog.read_with(cx, |d, _| d.needs_password() && !d.loading())
    });
    // real keystrokes into the focused field, Enter submits
    win(cx, &fx, |w, cx| {
        w.render_frame(cx);
        let input = dialog.read(cx).password_input().clone();
        input.update(cx, |s, cx| s.focus(w, cx));
        w.input("nope", cx);
        w.press("enter", cx);
    });
    wait_until(cx, "wrong password answered", |cx| {
        dialog.read_with(cx, |d, _| !d.loading() && d.error() == "启动密钥不正确")
    });
    assert_eq!(
        fake.attempts
            .lock()
            .unwrap()
            .last()
            .cloned()
            .flatten()
            .as_deref(),
        Some("nope")
    );
    // the button
    win(cx, &fx, |w, cx| {
        let input = dialog.read(cx).password_input().clone();
        input.update(cx, |s, cx| s.set_value("pw".to_string(), w, cx));
        w.render_frame(cx);
        w.click("vault-unlock", cx);
    });
    wait_until(cx, "unlocked by the button", |_| fake.is_unlocked());
}
