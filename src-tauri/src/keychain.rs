use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
#[cfg(not(target_os = "macos"))]
use keyring::{Entry, Error as KeyringError};
#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
#[cfg(target_os = "macos")]
use security_framework::os::macos::passwords::find_generic_password;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const SERVICE_NAME: &str = "com.opsbatch.app";
const KEY_API_KEY: &str = "ai_api_key";
const KEY_HOST_PASSWORD: &str = "host_password";
const KEY_HOST_PRIVATE_KEY: &str = "host_private_key";
const KEY_GITHUB_TOKEN: &str = "github_token";
const KEY_SSH_HOST_KEY: &str = "ssh_host_key";
const VAULT_FILE_NAME: &str = "opsbatch-secrets.vault.json";

static LOCAL_VAULT_DIR: OnceLock<PathBuf> = OnceLock::new();
static VAULT_FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    Missing,
    Backend(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "secure storage entry is missing"),
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

#[derive(Debug, Serialize, Deserialize)]
struct VaultFile {
    version: u8,
    entries: BTreeMap<String, VaultEntry>,
    #[serde(default)]
    deleted: BTreeSet<String>,
}

impl Default for VaultFile {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
            deleted: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct VaultEntry {
    encrypted: String,
}

#[derive(Debug, Clone)]
struct LocalSecretVault {
    path: PathBuf,
    key_path: PathBuf,
}

impl LocalSecretVault {
    fn new(path: PathBuf) -> Self {
        let key_path = path.with_extension("key");
        Self { path, key_path }
    }

    fn store(&self, kind: &str, id: &str, value: &str) -> Result<(), String> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| "本地加密存储锁已损坏".to_string())?;
        let account = secret_account(kind, id);
        let mut vault = self.read_vault_locked().map_err(String::from)?;
        vault.entries.insert(
            account.clone(),
            VaultEntry {
                encrypted: self.encrypt_value(&account, value)?,
            },
        );
        vault.deleted.remove(&account);
        self.write_vault_locked(&vault)
    }

    fn get(&self, kind: &str, id: &str) -> Result<String, SecretError> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| SecretError::Backend("本地加密存储锁已损坏".to_string()))?;
        let account = secret_account(kind, id);
        let vault = self.read_vault_locked()?;
        let entry = vault.entries.get(&account).ok_or(SecretError::Missing)?;
        self.decrypt_value(&account, &entry.encrypted)
    }

    fn delete(&self, kind: &str, id: &str) -> Result<(), String> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| "本地加密存储锁已损坏".to_string())?;
        let account = secret_account(kind, id);
        let mut vault = self.read_vault_locked().map_err(String::from)?;
        vault.entries.remove(&account);
        vault.deleted.insert(account);
        self.write_vault_locked(&vault)
    }

    fn has_delete_marker(&self, kind: &str, id: &str) -> Result<bool, SecretError> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| SecretError::Backend("本地加密存储锁已损坏".to_string()))?;
        let account = secret_account(kind, id);
        let vault = self.read_vault_locked()?;
        Ok(vault.deleted.contains(&account))
    }

    fn read_vault_locked(&self) -> Result<VaultFile, SecretError> {
        if !self.path.exists() {
            return Ok(VaultFile::default());
        }
        let text = fs::read_to_string(&self.path)
            .map_err(|e| SecretError::Backend(format!("本地加密存储读取失败: {}", e)))?;
        if text.trim().is_empty() {
            return Ok(VaultFile::default());
        }
        serde_json::from_str(&text)
            .map_err(|e| SecretError::Backend(format!("本地加密存储格式无效: {}", e)))
    }

    fn write_vault_locked(&self, vault: &VaultFile) -> Result<(), String> {
        let payload = serde_json::to_vec_pretty(vault)
            .map_err(|e| format!("本地加密存储序列化失败: {}", e))?;
        write_private_file(&self.path, &payload)
    }

    fn encrypt_value(&self, account: &str, value: &str) -> Result<String, String> {
        let key = self.read_or_create_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| format!("本地加密存储初始化失败: {}", e))?;
        let nonce_bytes = new_nonce();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: value.as_bytes(),
                    aad: account.as_bytes(),
                },
            )
            .map_err(|e| format!("本地加密存储加密失败: {}", e))?;
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&ciphertext);
        Ok(BASE64.encode(result))
    }

    fn decrypt_value(&self, account: &str, encrypted: &str) -> Result<String, SecretError> {
        let data = BASE64
            .decode(encrypted)
            .map_err(|e| SecretError::Backend(format!("本地加密存储解码失败: {}", e)))?;
        if data.len() < 12 {
            return Err(SecretError::Backend("本地加密存储数据长度无效".to_string()));
        }
        let key = self.read_or_create_key().map_err(SecretError::Backend)?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| SecretError::Backend(format!("本地加密存储初始化失败: {}", e)))?;
        let nonce = Nonce::from_slice(&data[..12]);
        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &data[12..],
                    aad: account.as_bytes(),
                },
            )
            .map_err(|e| SecretError::Backend(format!("本地加密存储解密失败: {}", e)))?;
        String::from_utf8(plaintext)
            .map_err(|e| SecretError::Backend(format!("本地加密存储 UTF-8 解码失败: {}", e)))
    }

    fn read_or_create_key(&self) -> Result<[u8; 32], String> {
        if self.key_path.exists() {
            let encoded = fs::read_to_string(&self.key_path)
                .map_err(|e| format!("本地加密密钥读取失败: {}", e))?;
            let bytes = BASE64
                .decode(encoded.trim())
                .map_err(|e| format!("本地加密密钥格式无效: {}", e))?;
            return bytes
                .try_into()
                .map_err(|_| "本地加密密钥长度无效".to_string());
        }

        let key = new_master_key();
        let encoded = BASE64.encode(key);
        write_private_file(&self.key_path, encoded.as_bytes())?;
        Ok(key)
    }
}

trait LegacySecretBackend {
    fn get(&self, kind: &str, id: &str) -> Result<String, SecretError>;
}

struct OsLegacySecretBackend;

impl LegacySecretBackend for OsLegacySecretBackend {
    fn get(&self, kind: &str, id: &str) -> Result<String, SecretError> {
        let account = secret_account(kind, id);
        legacy_get_secret(&account)
    }
}

pub(crate) fn init_local_vault_dir(app_data_dir: PathBuf) -> Result<(), String> {
    fs::create_dir_all(&app_data_dir).map_err(|e| format!("本地加密存储目录创建失败: {}", e))?;
    if let Some(existing) = LOCAL_VAULT_DIR.get() {
        if existing == &app_data_dir {
            return Ok(());
        }
        return Err(format!(
            "本地加密存储目录已初始化为 {}，不能切换到 {}",
            existing.display(),
            app_data_dir.display()
        ));
    }
    LOCAL_VAULT_DIR
        .set(app_data_dir)
        .map_err(|_| "本地加密存储目录初始化失败".to_string())
}

fn store_secret(kind: &str, id: &str, value: &str) -> Result<(), String> {
    let vault = default_vault();
    vault.store(kind, id, value)?;
    match vault.get(kind, id) {
        Ok(stored) if stored == value => Ok(()),
        Ok(_) => Err("本地加密存储写入后校验失败：读回内容不匹配".to_string()),
        Err(error) => Err(format!(
            "本地加密存储写入后校验失败 account={} error={}",
            secret_account(kind, id),
            error
        )),
    }
}

fn get_secret(kind: &str, id: &str) -> Result<String, SecretError> {
    let vault = default_vault();
    let legacy = OsLegacySecretBackend;
    get_secret_with_backends(&vault, &legacy, kind, id)
}

fn delete_secret(kind: &str, id: &str) -> Result<(), String> {
    default_vault().delete(kind, id)
}

fn secret_account(kind: &str, id: &str) -> String {
    format!("{}:{}", kind, id)
}

fn get_secret_with_backends(
    vault: &LocalSecretVault,
    legacy: &impl LegacySecretBackend,
    kind: &str,
    id: &str,
) -> Result<String, SecretError> {
    match vault.get(kind, id) {
        Ok(value) => Ok(value),
        Err(SecretError::Missing) => {
            if vault.has_delete_marker(kind, id)? {
                return Err(SecretError::Missing);
            }
            let legacy_value = legacy.get(kind, id)?;
            vault.store(kind, id, &legacy_value).map_err(|e| {
                SecretError::Backend(format!("旧凭据迁移到本地加密存储失败: {}", e))
            })?;
            Ok(legacy_value)
        }
        Err(error) => Err(error),
    }
}

fn default_vault() -> LocalSecretVault {
    LocalSecretVault::new(default_vault_dir().join(VAULT_FILE_NAME))
}

fn default_vault_dir() -> PathBuf {
    if let Some(path) = LOCAL_VAULT_DIR.get() {
        return path.clone();
    }
    if let Ok(path) = std::env::var("OPSBATCH_APP_DATA_DIR") {
        return PathBuf::from(path);
    }
    fallback_app_data_dir()
}

fn fallback_app_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("OpsBatch");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(app_data) = std::env::var("APPDATA") {
            return PathBuf::from(app_data).join("OpsBatch");
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg_data_home).join("opsbatch");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("opsbatch");
        }
    }
    PathBuf::from(".opsbatch")
}

fn new_master_key() -> [u8; 32] {
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(first.as_bytes());
    key[16..].copy_from_slice(second.as_bytes());
    key
}

fn new_nonce() -> [u8; 12] {
    let nonce = uuid::Uuid::new_v4();
    let mut bytes = [0u8; 12];
    bytes.copy_from_slice(&nonce.as_bytes()[..12]);
    bytes
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("目录创建失败: {}", e))?;
    }

    let temp_path = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);

    {
        let mut file = options
            .open(&temp_path)
            .map_err(|e| format!("文件写入失败: {}", e))?;
        file.write_all(bytes)
            .map_err(|e| format!("文件写入失败: {}", e))?;
        file.sync_all()
            .map_err(|e| format!("文件同步失败: {}", e))?;
    }

    fs::rename(&temp_path, path).map_err(|e| format!("文件替换失败: {}", e))
}

fn legacy_get_secret(account: &str) -> Result<String, SecretError> {
    #[cfg(target_os = "macos")]
    {
        return macos_get_secret(account);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let entry = Entry::new(SERVICE_NAME, account)
            .map_err(|e| SecretError::Backend(format!("keyring 创建失败: {}", e)))?;
        entry.get_password().map_err(|e| match e {
            KeyringError::NoEntry => SecretError::Missing,
            KeyringError::NoStorageAccess(_) => {
                SecretError::Backend(format!("keyring 读取失败（安全存储不可访问）: {}", e))
            }
            _ => SecretError::Backend(format!("keyring 读取失败: {}", e)),
        })
    }
}

#[cfg(target_os = "macos")]
fn macos_keychain() -> Result<SecKeychain, String> {
    SecKeychain::default_for_domain(SecPreferencesDomain::User)
        .map_err(|e| format!("macOS keychain 打开失败: {}", e))
}

#[cfg(target_os = "macos")]
fn macos_get_secret(account: &str) -> Result<String, SecretError> {
    let keychain = macos_keychain().map_err(SecretError::Backend)?;
    let (password, _) =
        find_generic_password(Some(&[keychain]), SERVICE_NAME, account).map_err(|e| {
            let text = e.to_string();
            if text.contains("-25300") || text.contains("could not be found") {
                SecretError::Missing
            } else {
                SecretError::Backend(format!("macOS keychain 读取失败: {}", text))
            }
        })?;
    String::from_utf8(password.to_vec())
        .map_err(|e| SecretError::Backend(format!("macOS keychain UTF-8 解码失败: {}", e)))
}

pub fn store_api_key(provider: &str, api_key: &str) -> Result<(), String> {
    store_secret(KEY_API_KEY, provider, api_key)
}

pub fn get_api_key(provider: &str) -> Result<String, SecretError> {
    get_secret(KEY_API_KEY, provider)
}

pub fn delete_api_key(provider: &str) -> Result<(), String> {
    delete_secret(KEY_API_KEY, provider)
}

pub fn store_host_password(host_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_HOST_PASSWORD, host_id, value)
}

pub fn get_host_password(host_id: &str) -> Result<String, SecretError> {
    get_secret(KEY_HOST_PASSWORD, host_id)
}

pub fn host_password_debug_label(host_id: &str) -> String {
    format!(
        "service={} account={}",
        SERVICE_NAME,
        secret_account(KEY_HOST_PASSWORD, host_id)
    )
}

pub fn delete_host_password(host_id: &str) -> Result<(), String> {
    delete_secret(KEY_HOST_PASSWORD, host_id)
}

pub fn store_host_private_key(host_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_HOST_PRIVATE_KEY, host_id, value)
}

pub fn get_host_private_key(host_id: &str) -> Result<String, SecretError> {
    get_secret(KEY_HOST_PRIVATE_KEY, host_id)
}

pub fn delete_host_private_key(host_id: &str) -> Result<(), String> {
    delete_secret(KEY_HOST_PRIVATE_KEY, host_id)
}

pub fn store_github_token(repo_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_GITHUB_TOKEN, repo_id, value)
}

pub fn get_github_token(repo_id: &str) -> Result<String, SecretError> {
    get_secret(KEY_GITHUB_TOKEN, repo_id)
}

pub fn delete_github_token(repo_id: &str) -> Result<(), String> {
    delete_secret(KEY_GITHUB_TOKEN, repo_id)
}

pub fn store_ssh_host_key(host_id: &str, fingerprint: &str) -> Result<(), String> {
    store_secret(KEY_SSH_HOST_KEY, host_id, fingerprint)
}

pub fn get_ssh_host_key(host_id: &str) -> Result<String, SecretError> {
    get_secret(KEY_SSH_HOST_KEY, host_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn temp_vault_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "opsbatch-vault-{}-{}.json",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn local_vault_round_trips_without_storing_plaintext() {
        let path = temp_vault_path("round-trip");
        let vault = LocalSecretVault::new(path.clone());

        vault
            .store("host_password", "host-a", "sensitive-password")
            .expect("store");

        let stored = std::fs::read_to_string(&path).expect("vault file");
        assert!(!stored.contains("sensitive-password"));
        assert_eq!(
            "sensitive-password",
            vault.get("host_password", "host-a").expect("get")
        );

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(vault.key_path);
    }

    #[test]
    fn local_vault_reports_missing_secret() {
        let path = temp_vault_path("missing");
        let vault = LocalSecretVault::new(path.clone());

        assert_eq!(
            SecretError::Missing,
            vault.get("host_password", "missing").unwrap_err()
        );

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(vault.key_path);
    }

    #[test]
    fn fallback_secret_is_migrated_into_local_vault() {
        let path = temp_vault_path("fallback");
        let vault = LocalSecretVault::new(path.clone());
        let legacy = MemoryLegacySecretBackend::new();
        legacy.store("host_password", "host-a", "legacy-password");

        let first = get_secret_with_backends(&vault, &legacy, "host_password", "host-a")
            .expect("fallback get");
        assert_eq!("legacy-password", first);
        assert_eq!(
            "legacy-password",
            vault.get("host_password", "host-a").expect("migrated")
        );

        legacy.clear();
        let second = get_secret_with_backends(&vault, &legacy, "host_password", "host-a")
            .expect("local get");
        assert_eq!("legacy-password", second);

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(vault.key_path);
    }

    #[derive(Clone, Default)]
    struct MemoryLegacySecretBackend {
        values: Arc<Mutex<HashMap<String, String>>>,
    }

    impl MemoryLegacySecretBackend {
        fn new() -> Self {
            Self::default()
        }

        fn store(&self, kind: &str, id: &str, value: &str) {
            self.values
                .lock()
                .expect("lock")
                .insert(secret_account(kind, id), value.to_string());
        }

        fn clear(&self) {
            self.values.lock().expect("lock").clear();
        }
    }

    impl LegacySecretBackend for MemoryLegacySecretBackend {
        fn get(&self, kind: &str, id: &str) -> Result<String, SecretError> {
            self.values
                .lock()
                .expect("lock")
                .get(&secret_account(kind, id))
                .cloned()
                .ok_or(SecretError::Missing)
        }
    }
}
