//! Secret storage facade used while the real encrypted vault is switched off (default build).
//!
//! The production implementation (local AES-GCM vault + OS keychain, `keychain_real.rs`) is behind the
//! `real-vault` cargo feature and is enabled last (TASKS M1-05): during development nothing here touches the
//! system keychain or asks for a master password. The API mirrors the real module 1:1 so callers do not change.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    Missing,
    Locked,
    Backend(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "secure storage entry is missing"),
            Self::Locked => write!(f, "本地加密 vault 未解锁，请先完成启动解锁。"),
            Self::Backend(message) => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for SecretError {}

impl From<SecretError> for String {
    fn from(value: SecretError) -> Self {
        value.to_string()
    }
}

fn store() -> &'static Mutex<HashMap<String, String>> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

fn put(kind: &str, id: &str, value: &str) -> Result<(), String> {
    store().lock().unwrap().insert(key(kind, id), value.to_string());
    Ok(())
}

fn get(kind: &str, id: &str) -> Result<String, SecretError> {
    store().lock().unwrap().get(&key(kind, id)).cloned().ok_or(SecretError::Missing)
}

fn del(kind: &str, id: &str) -> Result<(), String> {
    store().lock().unwrap().remove(&key(kind, id));
    Ok(())
}

pub(crate) fn init_local_vault_dir(_app_data_dir: PathBuf) -> Result<(), String> {
    Ok(())
}

pub fn is_local_vault_unlocked() -> bool {
    true
}

pub fn is_locked_error_message(message: &str) -> bool {
    message.contains("本地加密 vault 未解锁")
}

pub fn is_local_vault_unlocked_command() -> bool {
    true
}

pub fn unlock_local_vault(_master_key: Option<String>) -> Result<(), String> {
    Ok(())
}

pub fn store_api_key(provider: &str, api_key: &str) -> Result<(), String> { put("api_key", provider, api_key) }
pub fn get_api_key(provider: &str) -> Result<String, SecretError> { get("api_key", provider) }
pub fn delete_api_key(provider: &str) -> Result<(), String> { del("api_key", provider) }

pub fn store_host_password(host_id: &str, value: &str) -> Result<(), String> { put("host_password", host_id, value) }
pub fn get_host_password(host_id: &str) -> Result<String, SecretError> { get("host_password", host_id) }
pub fn host_password_debug_label(host_id: &str) -> String { format!("memory:host_password:{host_id}") }
pub fn delete_host_password(host_id: &str) -> Result<(), String> { del("host_password", host_id) }

pub fn store_host_private_key(host_id: &str, value: &str) -> Result<(), String> { put("host_private_key", host_id, value) }
pub fn get_host_private_key(host_id: &str) -> Result<String, SecretError> { get("host_private_key", host_id) }
pub fn delete_host_private_key(host_id: &str) -> Result<(), String> { del("host_private_key", host_id) }

pub fn store_host_vnc_password(host_id: &str, value: &str) -> Result<(), String> { put("host_vnc_password", host_id, value) }
pub fn get_host_vnc_password(host_id: &str) -> Result<String, SecretError> { get("host_vnc_password", host_id) }
pub fn delete_host_vnc_password(host_id: &str) -> Result<(), String> { del("host_vnc_password", host_id) }

pub fn store_host_proxy_password(host_id: &str, value: &str) -> Result<(), String> { put("host_proxy_password", host_id, value) }
pub fn get_host_proxy_password(host_id: &str) -> Result<String, SecretError> { get("host_proxy_password", host_id) }
pub fn delete_host_proxy_password(host_id: &str) -> Result<(), String> { del("host_proxy_password", host_id) }

pub fn store_github_token(repo_id: &str, value: &str) -> Result<(), String> { put("github_token", repo_id, value) }
pub fn get_github_token(repo_id: &str) -> Result<String, SecretError> { get("github_token", repo_id) }
pub fn delete_github_token(repo_id: &str) -> Result<(), String> { del("github_token", repo_id) }

pub fn store_ssh_host_key(host_id: &str, fingerprint: &str) -> Result<(), String> { put("ssh_host_key", host_id, fingerprint) }
pub fn get_ssh_host_key(host_id: &str) -> Result<String, SecretError> { get("ssh_host_key", host_id) }
