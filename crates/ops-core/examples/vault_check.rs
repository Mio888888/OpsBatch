//! Checks the encrypted vault against the REAL system keychain, one phase per process (so "restart" is a real restart).
//!
//!   OPSBATCH_KEYCHAIN_SERVICE=com.opsbatch.app.verify-123 OPSBATCH_VAULT_CHECK_DIR=/tmp/x \
//!     cargo run -p ops-core --features real-vault --example vault_check -- <phase>
//!
//! Phases: `setup` (first start: no key, set a startup password, store / read / delete secrets), `restart` (unlock from
//! the keychain alone, secrets survived), `stale` (the keychain key no longer opens the vault, the password does),
//! `nokeychain` / `nokeychain-restart` (a machine with no usable keychain, e.g. Linux without a Secret Service: the startup
//! password is asked, opens the vault, and is asked again after a restart; on a machine whose keychain works they report
//! SKIP), `cleanup` (delete the throw-away keychain item). `scripts/verify-real-vault.sh` runs them in order and always
//! cleans up (`--no-keychain` runs the two `nokeychain` phases instead).
//! It refuses to run against the default service: that item is shared with the Tauri build.

#[cfg(not(feature = "real-vault"))]
fn main() {
    eprintln!("build with --features real-vault");
    std::process::exit(2);
}

#[cfg(feature = "real-vault")]
fn main() {
    use ops_core::keychain as k;
    let phase = std::env::args().nth(1).unwrap_or_default();
    let service = k::system_keychain_service();
    if service == "com.opsbatch.app" {
        eprintln!(
            "refusing to run: set {} to a throw-away service name",
            k::SERVICE_OVERRIDE_ENV
        );
        std::process::exit(2);
    }
    let dir = std::env::var("OPSBATCH_VAULT_CHECK_DIR").unwrap_or_else(|_| {
        eprintln!("set OPSBATCH_VAULT_CHECK_DIR");
        std::process::exit(2)
    });
    let mut failures = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!(
            "{} {name}{}",
            if ok { "PASS" } else { "FAIL" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {detail}")
            }
        );
        if !ok {
            failures += 1;
        }
    };
    println!(
        "phase={phase} service={service} account={}",
        k::verification::master_key_account()
    );

    if phase == "cleanup" {
        let r = k::verification::delete_system_master_key();
        check("master-key item deleted", r.is_ok(), format!("{r:?}"));
    } else {
        // Core::open registers the vault directory (a fresh process each phase)
        ops_core::Core::open(&dir).expect("open data dir");
        match phase.as_str() {
            "setup" => {
                check(
                    "starts locked",
                    !k::is_local_vault_unlocked(),
                    String::new(),
                );
                let r = k::unlock_local_vault(None);
                check(
                    "no keychain key: asks for a startup password",
                    r.as_ref()
                        .err()
                        .is_some_and(|e| e.contains("本地加密密钥不存在")),
                    format!("{r:?}"),
                );
                check(
                    "still locked after the refusal",
                    !k::is_local_vault_unlocked(),
                    String::new(),
                );
                let locked = k::store_host_password("h1", "x");
                check(
                    "storing while locked is refused",
                    locked.is_err(),
                    format!("{locked:?}"),
                );
                let r = k::unlock_local_vault(Some("pw-1".into()));
                check(
                    "unlock with a new startup password",
                    r.is_ok() && k::is_local_vault_unlocked(),
                    format!("{r:?}"),
                );
                check(
                    "store host password",
                    k::store_host_password("h1", "hp-secret").is_ok(),
                    String::new(),
                );
                check(
                    "store api key",
                    k::store_api_key("p1", "sk-secret-123").is_ok(),
                    String::new(),
                );
                check(
                    "store github token",
                    k::store_github_token("r1", "ghp_secret").is_ok(),
                    String::new(),
                );
                check(
                    "read back host password",
                    k::get_host_password("h1").as_deref() == Ok("hp-secret"),
                    String::new(),
                );
                check(
                    "read back api key",
                    k::get_api_key("p1").as_deref() == Ok("sk-secret-123"),
                    String::new(),
                );
                check(
                    "delete + missing",
                    k::delete_github_token("r1").is_ok() && k::get_github_token("r1").is_err(),
                    String::new(),
                );
                let vault = std::fs::read_to_string(
                    std::path::Path::new(&dir).join("opsbatch-secrets.vault.json"),
                )
                .unwrap_or_default();
                check(
                    "vault file exists and holds no plaintext",
                    !vault.is_empty()
                        && !vault.contains("hp-secret")
                        && !vault.contains("sk-secret-123"),
                    format!("{} bytes", vault.len()),
                );
            }
            "restart" => {
                check(
                    "starts locked",
                    !k::is_local_vault_unlocked(),
                    String::new(),
                );
                let r = k::unlock_local_vault(None);
                check(
                    "unlocks from the system keychain alone",
                    r.is_ok() && k::is_local_vault_unlocked(),
                    format!("{r:?}"),
                );
                check(
                    "host password survived the restart",
                    k::get_host_password("h1").as_deref() == Ok("hp-secret"),
                    String::new(),
                );
                check(
                    "api key survived the restart",
                    k::get_api_key("p1").as_deref() == Ok("sk-secret-123"),
                    String::new(),
                );
            }
            "stale" => {
                let r = k::verification::replace_system_master_key_with_random();
                check(
                    "keychain key replaced by a wrong one",
                    r.is_ok(),
                    format!("{r:?}"),
                );
                let r = k::unlock_local_vault(None);
                check(
                    "stale key is reported",
                    r.as_ref()
                        .err()
                        .is_some_and(|e| e.contains("系统钥匙串中的本地加密密钥无法解开")),
                    format!("{r:?}"),
                );
                check("still locked", !k::is_local_vault_unlocked(), String::new());
                let r = k::unlock_local_vault(Some("wrong-password".into()));
                check(
                    "a wrong startup password is refused",
                    r.is_err() && !k::is_local_vault_unlocked(),
                    format!("{r:?}"),
                );
                let r = k::unlock_local_vault(Some("pw-1".into()));
                check(
                    "the startup password recovers it",
                    r.is_ok() && k::get_host_password("h1").as_deref() == Ok("hp-secret"),
                    format!("{r:?}"),
                );
            }
            "nokeychain" | "nokeychain-restart" => {
                let restart = phase == "nokeychain-restart";
                let r = k::unlock_local_vault(None);
                let works =
                    r.as_ref().err().is_some_and(|e| {
                        e.contains("本地加密密钥不存在") || e.contains("无法解开")
                    }) || r.is_ok();
                if works {
                    println!(
                        "SKIP the system keychain works on this machine, nothing to check here"
                    );
                } else {
                    check(
                        "keychain unavailable: the startup password is asked, with the reason",
                        r.as_ref()
                            .err()
                            .is_some_and(|e| e.contains(k::KEYCHAIN_UNAVAILABLE)),
                        format!("{r:?}"),
                    );
                    check(
                        "still locked after the refusal",
                        !k::is_local_vault_unlocked(),
                        String::new(),
                    );
                    if restart {
                        let r = k::unlock_local_vault(Some("wrong-password".into()));
                        check(
                            "a wrong startup password is refused",
                            r.is_err() && !k::is_local_vault_unlocked(),
                            format!("{r:?}"),
                        );
                    }
                    let r = k::unlock_local_vault(Some("pw-1".into()));
                    check(
                        "the startup password unlocks without a keychain",
                        r.is_ok() && k::is_local_vault_unlocked(),
                        format!("{r:?}"),
                    );
                    if restart {
                        check(
                            "the secret survived the restart",
                            k::get_host_password("h1").as_deref() == Ok("hp-secret"),
                            String::new(),
                        );
                    } else {
                        check(
                            "store host password",
                            k::store_host_password("h1", "hp-secret").is_ok(),
                            String::new(),
                        );
                        check(
                            "read it back",
                            k::get_host_password("h1").as_deref() == Ok("hp-secret"),
                            String::new(),
                        );
                    }
                }
            }
            other => {
                eprintln!("unknown phase {other}");
                std::process::exit(2);
            }
        }
    }
    println!(
        "{}",
        if failures == 0 {
            "RESULT ok"
        } else {
            "RESULT FAILED"
        }
    );
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
