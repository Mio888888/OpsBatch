//! The local vault in the UI: who unlocks it, when, and what waits for it (port of `keychainNotice.ts`).
//!
//! `request_unlock` is `requestKeychainNotice()`: every caller shares one pending unlock and one dialog
//! (`vault_dialog.rs`). Work that needs the secrets (loading AI preferences, pulling repositories on startup) registers
//! with `on_unlocked`, so nothing reads a locked vault and then writes the blanks back.

use std::sync::Arc;

use futures::channel::oneshot;
use gpui_kit::*;
use ops_core::Database;

/// What the dialog asks of the vault. The real one talks to the system keychain and the encrypted file; tests inject
/// their own so no keychain prompt ever opens.
pub trait VaultBackend: Send + Sync + 'static {
    fn is_unlocked(&self) -> bool;
    /// Unlocks with what the system keychain holds, or with the startup password the user typed.
    fn unlock(&self, master_key: Option<String>) -> Result<(), String>;
    /// Runs once after an unlock: plaintext secrets that older builds left in the database move into the vault.
    fn after_unlock(&self, db: &Database) -> Result<(), String>;
}

/// `ops_core::keychain` (the in-memory facade in the default build, the encrypted vault with `real-vault`).
pub struct SystemVault;

impl VaultBackend for SystemVault {
    fn is_unlocked(&self) -> bool {
        ops_core::keychain::is_local_vault_unlocked_command()
    }
    fn unlock(&self, master_key: Option<String>) -> Result<(), String> {
        ops_core::keychain::unlock_local_vault(master_key)
    }
    fn after_unlock(&self, db: &Database) -> Result<(), String> {
        db.migrate_plaintext_secrets_to_vault()
    }
}

type Waiter = Box<dyn FnOnce(&mut App)>;

pub struct Vault {
    backend: Arc<dyn VaultBackend>,
    waiters: Vec<Waiter>,
    requests: Vec<oneshot::Sender<bool>>,
    dialog_open: bool,
    dialog: Option<WeakEntity<crate::vault_dialog::VaultDialog>>,
}

impl Global for Vault {}

pub fn init(cx: &mut App) {
    #[allow(unused_mut)]
    let mut backend: Arc<dyn VaultBackend> = Arc::new(SystemVault);
    // Debug builds only: `OPSBATCH_VAULT_DEMO=password|stale` shows the unlock dialog without touching the system
    // keychain (the startup password is `demo`).
    #[cfg(debug_assertions)]
    if let Ok(mode) = std::env::var("OPSBATCH_VAULT_DEMO") {
        backend = Arc::new(DemoVault { stale: mode == "stale", unlocked: std::sync::atomic::AtomicBool::new(false) });
    }
    cx.set_global(Vault { backend, waiters: Vec::new(), requests: Vec::new(), dialog_open: false, dialog: None });
}

#[cfg(debug_assertions)]
struct DemoVault {
    stale: bool,
    unlocked: std::sync::atomic::AtomicBool,
}

#[cfg(debug_assertions)]
impl VaultBackend for DemoVault {
    fn is_unlocked(&self) -> bool {
        self.unlocked.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn unlock(&self, master_key: Option<String>) -> Result<(), String> {
        std::thread::sleep(std::time::Duration::from_millis(600));
        match master_key.as_deref().map(str::trim) {
            None | Some("") if self.stale => Err("系统钥匙串中的本地加密密钥无法解开现有 vault，请输入启动密钥。".into()),
            None | Some("") => Err("本地加密密钥不存在，请输入新的解锁密钥。".into()),
            Some("demo") => {
                self.unlocked.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            Some(_) => Err("启动密钥不正确".into()),
        }
    }
    fn after_unlock(&self, _db: &Database) -> Result<(), String> {
        Ok(())
    }
}

/// Replaces the backend (tests).
pub fn set_backend(backend: Arc<dyn VaultBackend>, cx: &mut App) {
    cx.global_mut::<Vault>().backend = backend;
}

pub fn backend(cx: &App) -> Arc<dyn VaultBackend> {
    cx.global::<Vault>().backend.clone()
}

pub fn is_unlocked(cx: &App) -> bool {
    cx.try_global::<Vault>().map(|v| v.backend.is_unlocked()).unwrap_or(true)
}

/// Runs `f` now when the vault is open, else right after it gets unlocked.
pub fn on_unlocked(cx: &mut App, f: impl FnOnce(&mut App) + 'static) {
    if is_unlocked(cx) {
        f(cx);
    } else {
        cx.global_mut::<Vault>().waiters.push(Box::new(f));
    }
}

/// Asks for the vault to be unlocked, opening the dialog on `window` unless one is already showing. Resolves `true`
/// once unlocked.
pub fn request_unlock(window: &mut Window, cx: &mut App) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    if is_unlocked(cx) {
        let _ = tx.send(true);
        return rx;
    }
    let open_dialog = {
        let v = cx.global_mut::<Vault>();
        v.requests.push(tx);
        !std::mem::replace(&mut v.dialog_open, true)
    };
    if open_dialog {
        let dialog = crate::vault_dialog::VaultDialog::open(window, cx);
        cx.global_mut::<Vault>().dialog = Some(dialog.downgrade());
    }
    rx
}

/// The dialog that is showing, if any.
pub fn dialog(cx: &App) -> Option<Entity<crate::vault_dialog::VaultDialog>> {
    cx.try_global::<Vault>().and_then(|v| v.dialog.as_ref()).and_then(|d| d.upgrade())
}

pub(crate) fn dialog_closed(cx: &mut App) {
    cx.global_mut::<Vault>().dialog_open = false;
}

/// Called by the dialog once the vault is open: answers every request and runs what was waiting.
pub(crate) fn unlocked(cx: &mut App) {
    let (requests, waiters) = {
        let v = cx.global_mut::<Vault>();
        (std::mem::take(&mut v.requests), std::mem::take(&mut v.waiters))
    };
    for tx in requests {
        let _ = tx.send(true);
    }
    for w in waiters {
        w(cx);
    }
}
