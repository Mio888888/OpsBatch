use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
#[cfg(not(test))]
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
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

const SERVICE_NAME: &str = "com.opsbatch.app";
const KEY_API_KEY: &str = "ai_api_key";
const KEY_HOST_PASSWORD: &str = "host_password";
const KEY_HOST_PRIVATE_KEY: &str = "host_private_key";
const KEY_HOST_VNC_PASSWORD: &str = "host_vnc_password";
const KEY_HOST_PROXY_PASSWORD: &str = "host_proxy_password";
const KEY_GITHUB_TOKEN: &str = "github_token";
const KEY_SSH_HOST_KEY: &str = "ssh_host_key";
#[cfg(not(test))]
const KEY_LOCAL_VAULT_MASTER_KEY: &str = "local_vault_master_key";
const VAULT_FILE_NAME: &str = "opsbatch-secrets.vault.json";
const LEGACY_VAULT_KEY_EXTENSION: &str = "key";
const SYSTEM_VAULT_MASTER_KEY_PREFIX: &str = "opsbatch-vault-master-key-v1:";

static LOCAL_VAULT_DIR: OnceLock<PathBuf> = OnceLock::new();
static VAULT_FILE_LOCK: Mutex<()> = Mutex::new(());
static VAULT_MASTER_KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);
#[cfg(test)]
static TEST_SYSTEM_KEYRING_VALUE: Mutex<Option<String>> = Mutex::new(None);
#[cfg(test)]
static TEST_SYSTEM_KEYRING_READS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static TEST_VAULT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SystemVaultMasterKey {
    key: [u8; 32],
    needs_user_setup: bool,
}

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
}

impl LocalSecretVault {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn store(&self, kind: &str, id: &str, value: &str) -> Result<(), String> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| "本地加密存储锁已损坏".to_string())?;
        let key = self.read_or_create_key().map_err(String::from)?;
        let account = secret_account(kind, id);
        let mut vault = self.read_vault_locked().map_err(String::from)?;
        vault.entries.insert(
            account.clone(),
            VaultEntry {
                encrypted: self.encrypt_value(&account, value, &key)?,
            },
        );
        vault.deleted.remove(&account);
        self.write_vault_locked(&vault)
    }

    fn get(&self, kind: &str, id: &str) -> Result<String, SecretError> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| SecretError::Backend("本地加密存储锁已损坏".to_string()))?;
        let key = self.read_or_create_key()?;
        let account = secret_account(kind, id);
        let vault = self.read_vault_locked()?;
        let entry = vault.entries.get(&account).ok_or(SecretError::Missing)?;
        decrypt_with_key(&account, &entry.encrypted, &key)
    }

    fn delete(&self, kind: &str, id: &str) -> Result<(), String> {
        let _guard = VAULT_FILE_LOCK
            .lock()
            .map_err(|_| "本地加密存储锁已损坏".to_string())?;
        self.read_or_create_key().map_err(String::from)?;
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
        self.read_or_create_key()?;
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

    fn encrypt_value(&self, account: &str, value: &str, key: &[u8; 32]) -> Result<String, String> {
        let cipher =
            Aes256Gcm::new_from_slice(key).map_err(|e| format!("本地加密存储初始化失败: {}", e))?;
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

    fn read_or_create_key(&self) -> Result<[u8; 32], SecretError> {
        current_vault_master_key()
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

pub fn is_local_vault_unlocked() -> bool {
    VAULT_MASTER_KEY
        .lock()
        .map(|key| key.is_some())
        .unwrap_or(false)
}

pub fn is_locked_error_message(message: &str) -> bool {
    message == SecretError::Locked.to_string()
}

#[tauri::command]
pub fn is_local_vault_unlocked_command() -> bool {
    is_local_vault_unlocked()
}

#[tauri::command]
pub fn unlock_local_vault(master_key: Option<String>) -> Result<(), String> {
    if is_local_vault_unlocked() {
        return Ok(());
    }

    match read_system_vault_master_key()? {
        Some(system_key) if !system_key.needs_user_setup => {
            unlock_with_existing_or_provided_key(system_key.key, master_key.as_deref())
        }
        Some(system_key) => unlock_with_setup_required_key(system_key.key, master_key.as_deref()),
        None => {
            let provided = master_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "本地加密密钥不存在，请输入新的解锁密钥。".to_string())?;
            let key = derive_master_key_from_passphrase(provided);
            validate_vault_key(&key)?;
            write_system_vault_master_key(&key)?;
            cache_vault_master_key(key)
        }
    }
}

fn unlock_with_setup_required_key(
    setup_key: [u8; 32],
    master_key: Option<&str>,
) -> Result<(), String> {
    let provided = master_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "本地加密密钥不存在，请输入新的解锁密钥。".to_string())?;
    let key = derive_master_key_from_passphrase(provided);
    match validate_vault_key(&setup_key) {
        Ok(()) => reencrypt_vault(&setup_key, &key)?,
        Err(_) => validate_vault_key(&key)?,
    }
    write_system_vault_master_key(&key)?;
    delete_legacy_vault_key_file()?;
    cache_vault_master_key(key)
}

fn unlock_with_existing_or_provided_key(
    existing_key: [u8; 32],
    master_key: Option<&str>,
) -> Result<(), String> {
    match validate_vault_key(&existing_key) {
        Ok(()) => cache_vault_master_key(existing_key),
        Err(_) => {
            let provided = master_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "系统钥匙串中的本地加密密钥无法解开现有 vault，请输入启动密钥。".to_string()
                })?;
            let key = derive_master_key_from_passphrase(provided);
            validate_vault_key(&key)?;
            write_system_vault_master_key(&key)?;
            cache_vault_master_key(key)
        }
    }
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

fn default_vault_path() -> PathBuf {
    default_vault_dir().join(VAULT_FILE_NAME)
}

fn legacy_vault_key_path() -> PathBuf {
    default_vault_path().with_extension(LEGACY_VAULT_KEY_EXTENSION)
}

fn default_vault_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Ok(dir) = TEST_VAULT_DIR.lock() {
            if let Some(path) = dir.as_ref() {
                return path.clone();
            }
        }
    }
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

fn current_vault_master_key() -> Result<[u8; 32], SecretError> {
    VAULT_MASTER_KEY
        .lock()
        .map_err(|_| SecretError::Backend("本地加密密钥缓存锁已损坏".to_string()))?
        .ok_or(SecretError::Locked)
}

fn cache_vault_master_key(key: [u8; 32]) -> Result<(), String> {
    let mut cached = VAULT_MASTER_KEY
        .lock()
        .map_err(|_| "本地加密密钥缓存锁已损坏".to_string())?;
    *cached = Some(key);
    Ok(())
}

fn derive_master_key_from_passphrase(passphrase: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"opsbatch-local-vault-master-key-v1");
    hasher.update(passphrase.as_bytes());
    hasher.finalize().into()
}

fn validate_vault_key(key: &[u8; 32]) -> Result<(), String> {
    let vault_path = default_vault_path();
    if !vault_path.exists() {
        return Ok(());
    }

    let text =
        fs::read_to_string(&vault_path).map_err(|e| format!("本地加密存储读取失败: {}", e))?;
    if text.trim().is_empty() {
        return Ok(());
    }

    let vault: VaultFile =
        serde_json::from_str(&text).map_err(|e| format!("本地加密存储格式无效: {}", e))?;
    let Some((account, entry)) = vault.entries.iter().next() else {
        return Ok(());
    };

    decrypt_with_key(account, &entry.encrypted, key)
        .map(|_| ())
        .map_err(|_| "本地加密密钥无法解开现有 vault，请检查输入或系统钥匙串记录。".to_string())
}

fn reencrypt_vault(old_key: &[u8; 32], new_key: &[u8; 32]) -> Result<(), String> {
    if old_key == new_key {
        return Ok(());
    }

    let vault = default_vault();
    let _guard = VAULT_FILE_LOCK
        .lock()
        .map_err(|_| "本地加密存储锁已损坏".to_string())?;
    let mut vault_file = vault.read_vault_locked().map_err(String::from)?;
    if vault_file.entries.is_empty() {
        return Ok(());
    }

    for (account, entry) in vault_file.entries.iter_mut() {
        let plaintext = decrypt_with_key(account, &entry.encrypted, old_key).map_err(|_| {
            "旧本地加密密钥无法解开现有 vault，请检查输入或系统钥匙串记录。".to_string()
        })?;
        entry.encrypted = vault.encrypt_value(account, &plaintext, new_key)?;
    }
    vault.write_vault_locked(&vault_file)
}

fn decrypt_with_key(account: &str, encrypted: &str, key: &[u8; 32]) -> Result<String, SecretError> {
    let data = BASE64
        .decode(encrypted)
        .map_err(|e| SecretError::Backend(format!("本地加密存储解码失败: {}", e)))?;
    if data.len() < 12 {
        return Err(SecretError::Backend("本地加密存储数据长度无效".to_string()));
    }
    let cipher = Aes256Gcm::new_from_slice(key)
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

#[allow(clippy::needless_return)] // cfg(test)/cfg(not(test)) 分支下，test 构建去掉后半块后需显式 return 才不退化为单元值
fn read_system_vault_master_key() -> Result<Option<SystemVaultMasterKey>, String> {
    #[cfg(test)]
    {
        return test_system_keyring_get();
    }

    #[cfg(not(test))]
    {
        let account = secret_account(KEY_LOCAL_VAULT_MASTER_KEY, "default");
        let entry =
            Entry::new(SERVICE_NAME, &account).map_err(|e| format!("keyring 创建失败: {}", e))?;
        match entry.get_password() {
            Ok(encoded) => decode_system_vault_master_key(&encoded).map(Some),
            Err(KeyringError::NoEntry) => read_legacy_system_vault_master_key(),
            Err(KeyringError::NoStorageAccess(_)) => {
                Err("系统钥匙串不可访问，无法解锁本地加密存储。".to_string())
            }
            Err(e) => Err(format!("系统钥匙串读取失败: {}", e)),
        }
    }
}

#[allow(clippy::needless_return)] // 同 read_system_vault_master_key：cfg 分支需显式 return
fn write_system_vault_master_key(key: &[u8; 32]) -> Result<(), String> {
    #[cfg(test)]
    {
        return test_system_keyring_set(&encode_system_vault_master_key(key));
    }

    #[cfg(not(test))]
    {
        let account = secret_account(KEY_LOCAL_VAULT_MASTER_KEY, "default");
        let entry =
            Entry::new(SERVICE_NAME, &account).map_err(|e| format!("keyring 创建失败: {}", e))?;
        entry
            .set_password(&encode_system_vault_master_key(key))
            .map_err(|e| format!("系统钥匙串保存失败: {}", e))
    }
}

fn read_legacy_system_vault_master_key() -> Result<Option<SystemVaultMasterKey>, String> {
    if !legacy_vault_key_path().exists() {
        return Ok(None);
    }
    read_legacy_vault_key().map(|key| {
        Some(SystemVaultMasterKey {
            key,
            needs_user_setup: true,
        })
    })
}

fn read_legacy_vault_key() -> Result<[u8; 32], String> {
    let encoded = fs::read_to_string(legacy_vault_key_path())
        .map_err(|e| format!("旧本地加密密钥读取失败: {}", e))?;
    decode_master_key(&encoded)
}

fn delete_legacy_vault_key_file() -> Result<(), String> {
    let path = legacy_vault_key_path();
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("旧本地加密密钥删除失败: {}", error)),
    }
}

fn encode_system_vault_master_key(key: &[u8; 32]) -> String {
    format!("{}{}", SYSTEM_VAULT_MASTER_KEY_PREFIX, BASE64.encode(key))
}

fn decode_system_vault_master_key(encoded: &str) -> Result<SystemVaultMasterKey, String> {
    let trimmed = encoded.trim();
    if let Some(value) = trimmed.strip_prefix(SYSTEM_VAULT_MASTER_KEY_PREFIX) {
        return decode_system_keyring_master_key(value);
    }
    decode_system_keyring_master_key(trimmed)
}

fn decode_system_keyring_master_key(encoded: &str) -> Result<SystemVaultMasterKey, String> {
    decode_master_key(encoded).map(|key| SystemVaultMasterKey {
        key,
        needs_user_setup: false,
    })
}

fn decode_master_key(encoded: &str) -> Result<[u8; 32], String> {
    let bytes = BASE64
        .decode(encoded.trim())
        .map_err(|e| format!("系统钥匙串密钥格式无效: {}", e))?;
    bytes
        .try_into()
        .map_err(|_| "系统钥匙串密钥长度无效".to_string())
}

#[cfg(test)]
fn test_system_keyring_get() -> Result<Option<SystemVaultMasterKey>, String> {
    TEST_SYSTEM_KEYRING_READS.fetch_add(1, Ordering::SeqCst);
    let value = TEST_SYSTEM_KEYRING_VALUE
        .lock()
        .map_err(|_| "测试系统钥匙串锁已损坏".to_string())?
        .clone();
    if let Some(value) = value {
        return decode_system_vault_master_key(&value).map(Some);
    }
    read_legacy_system_vault_master_key()
}

#[cfg(test)]
fn raw_test_system_keyring_get() -> Result<Option<String>, String> {
    TEST_SYSTEM_KEYRING_VALUE
        .lock()
        .map_err(|_| "测试系统钥匙串锁已损坏".to_string())
        .map(|value| value.clone())
}

#[cfg(test)]
fn test_system_keyring_set(encoded: &str) -> Result<(), String> {
    let mut value = TEST_SYSTEM_KEYRING_VALUE
        .lock()
        .map_err(|_| "测试系统钥匙串锁已损坏".to_string())?;
    *value = Some(encoded.to_string());
    Ok(())
}

#[cfg(test)]
fn clear_cached_vault_key_for_tests() {
    if let Ok(mut key) = VAULT_MASTER_KEY.lock() {
        *key = None;
    }
    if let Ok(mut value) = TEST_SYSTEM_KEYRING_VALUE.lock() {
        *value = None;
    }
    clear_mock_system_keyring_reads_for_tests();
}

#[cfg(test)]
fn set_test_vault_dir_for_tests(path: Option<PathBuf>) {
    if let Ok(mut dir) = TEST_VAULT_DIR.lock() {
        *dir = path;
    }
}

#[cfg(test)]
fn unlock_local_vault_for_tests(key: [u8; 32]) {
    cache_vault_master_key(key).expect("cache test vault key");
}

#[cfg(test)]
fn store_mock_system_keyring_key_for_tests(key: [u8; 32]) {
    test_system_keyring_set(&encode_system_vault_master_key(&key)).expect("store test keyring key");
}

#[cfg(test)]
fn store_mock_legacy_system_keyring_key_for_tests(key: [u8; 32]) {
    test_system_keyring_set(&BASE64.encode(key)).expect("store test keyring key");
}

#[cfg(test)]
fn clear_mock_system_keyring_reads_for_tests() {
    TEST_SYSTEM_KEYRING_READS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
fn mock_system_keyring_read_count_for_tests() -> usize {
    TEST_SYSTEM_KEYRING_READS.load(Ordering::SeqCst)
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
        macos_get_secret(account)
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

pub fn store_host_vnc_password(host_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_HOST_VNC_PASSWORD, host_id, value)
}

pub fn get_host_vnc_password(host_id: &str) -> Result<String, SecretError> {
    get_secret(KEY_HOST_VNC_PASSWORD, host_id)
}

pub fn delete_host_vnc_password(host_id: &str) -> Result<(), String> {
    delete_secret(KEY_HOST_VNC_PASSWORD, host_id)
}

pub fn store_host_proxy_password(host_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_HOST_PROXY_PASSWORD, host_id, value)
}

pub fn get_host_proxy_password(host_id: &str) -> Result<String, SecretError> {
    get_secret(KEY_HOST_PROXY_PASSWORD, host_id)
}

pub fn delete_host_proxy_password(host_id: &str) -> Result<(), String> {
    delete_secret(KEY_HOST_PROXY_PASSWORD, host_id)
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

    static TEST_GUARD: Mutex<()> = Mutex::new(());

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

    fn temp_vault_dir(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "opsbatch-vault-dir-{}-{}",
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
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        unlock_local_vault_for_tests([3; 32]);
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
    }

    #[test]
    fn local_vault_reports_missing_secret() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        unlock_local_vault_for_tests([4; 32]);
        let path = temp_vault_path("missing");
        let vault = LocalSecretVault::new(path.clone());

        assert_eq!(
            SecretError::Missing,
            vault.get("host_password", "missing").unwrap_err()
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn fallback_secret_is_migrated_into_local_vault() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        unlock_local_vault_for_tests([5; 32]);
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
    }

    #[test]
    fn local_vault_requires_unlock_before_use() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let path = temp_vault_path("locked");
        let vault = LocalSecretVault::new(path.clone());

        assert!(vault.store("host_password", "host-a", "secret").is_err());
        assert!(matches!(
            vault.get("host_password", "host-a"),
            Err(SecretError::Locked)
        ));
        assert!(matches!(
            vault.delete("host_password", "host-a"),
            Err(message) if is_locked_error_message(&message)
        ));
        assert!(matches!(
            vault.has_delete_marker("host_password", "host-a"),
            Err(SecretError::Locked)
        ));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn local_vault_uses_cached_key_after_unlock() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let path = temp_vault_path("unlocked");
        let vault = LocalSecretVault::new(path.clone());

        unlock_local_vault_for_tests([7; 32]);
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");
        clear_mock_system_keyring_reads_for_tests();

        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));
        assert_eq!(0, mock_system_keyring_read_count_for_tests());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn host_vnc_and_proxy_passwords_round_trip_through_local_vault() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let dir = temp_vault_dir("embedded-host-secrets");
        set_test_vault_dir_for_tests(Some(dir.clone()));
        unlock_local_vault_for_tests([9; 32]);

        store_host_vnc_password("host-a", "vnc-secret").expect("store vnc");
        store_host_proxy_password("host-a", "proxy-secret").expect("store proxy");

        assert_eq!(
            "vnc-secret",
            get_host_vnc_password("host-a").expect("get vnc")
        );
        assert_eq!(
            "proxy-secret",
            get_host_proxy_password("host-a").expect("get proxy")
        );

        let _ = std::fs::remove_dir_all(dir);
        set_test_vault_dir_for_tests(None);
    }

    #[test]
    fn unlock_local_vault_reads_system_keyring_once_when_present() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        store_mock_system_keyring_key_for_tests([11; 32]);

        unlock_local_vault(None).expect("unlock");
        assert!(is_local_vault_unlocked());
        assert_eq!(1, mock_system_keyring_read_count_for_tests());

        let path = temp_vault_path("system-keyring");
        let vault = LocalSecretVault::new(path.clone());
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");
        clear_mock_system_keyring_reads_for_tests();
        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));
        assert_eq!(0, mock_system_keyring_read_count_for_tests());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unlock_local_vault_requires_passphrase_when_system_keyring_missing() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();

        let error = unlock_local_vault(None).unwrap_err();
        assert!(error.contains("本地加密密钥不存在"));

        unlock_local_vault(Some("startup-secret".to_string())).expect("unlock");
        assert!(is_local_vault_unlocked());

        let path = temp_vault_path("passphrase");
        let vault = LocalSecretVault::new(path.clone());
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");
        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unlock_local_vault_requires_passphrase_when_only_legacy_key_file_exists() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let dir = temp_vault_dir("legacy-key-only");
        let path = dir.join(VAULT_FILE_NAME);
        let vault = LocalSecretVault::new(path.clone());
        let legacy_key_path = dir
            .join(VAULT_FILE_NAME)
            .with_extension(LEGACY_VAULT_KEY_EXTENSION);
        let legacy_key = [42; 32];

        std::fs::create_dir_all(&dir).expect("vault dir");
        set_test_vault_dir_for_tests(Some(dir.clone()));
        unlock_local_vault_for_tests(legacy_key);
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");
        clear_cached_vault_key_for_tests();
        std::fs::write(&legacy_key_path, BASE64.encode(legacy_key)).expect("legacy key");

        let error = unlock_local_vault(None).unwrap_err();
        assert!(error.contains("本地加密密钥不存在"));
        assert!(!is_local_vault_unlocked());
        assert_eq!(None, raw_test_system_keyring_get().expect("test keyring"));

        unlock_local_vault(Some("startup-secret".to_string())).expect("unlock");
        assert!(is_local_vault_unlocked());
        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));
        assert!(raw_test_system_keyring_get()
            .expect("raw test keyring")
            .expect("stored key")
            .starts_with(SYSTEM_VAULT_MASTER_KEY_PREFIX));
        assert!(!legacy_key_path.exists());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(legacy_key_path);
        let _ = std::fs::remove_dir(dir);
        set_test_vault_dir_for_tests(None);
    }

    #[test]
    fn unlock_local_vault_falls_back_to_passphrase_when_legacy_key_file_is_stale() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let dir = temp_vault_dir("stale-legacy-key");
        let path = dir.join(VAULT_FILE_NAME);
        let vault = LocalSecretVault::new(path.clone());
        let passphrase = "startup-secret";
        let correct_key = derive_master_key_from_passphrase(passphrase);
        let stale_legacy_key = [88; 32];
        let legacy_key_path = dir
            .join(VAULT_FILE_NAME)
            .with_extension(LEGACY_VAULT_KEY_EXTENSION);

        std::fs::create_dir_all(&dir).expect("vault dir");
        set_test_vault_dir_for_tests(Some(dir.clone()));
        unlock_local_vault_for_tests(correct_key);
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");

        clear_cached_vault_key_for_tests();
        std::fs::write(&legacy_key_path, BASE64.encode(stale_legacy_key)).expect("legacy key");

        let error = unlock_local_vault(None).unwrap_err();
        assert!(error.contains("本地加密密钥不存在"));
        assert!(!is_local_vault_unlocked());

        unlock_local_vault(Some(passphrase.to_string())).expect("unlock");
        assert!(is_local_vault_unlocked());
        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));
        assert!(raw_test_system_keyring_get()
            .expect("raw test keyring")
            .expect("stored key")
            .starts_with(SYSTEM_VAULT_MASTER_KEY_PREFIX));
        assert!(!legacy_key_path.exists());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(legacy_key_path);
        let _ = std::fs::remove_dir(dir);
        set_test_vault_dir_for_tests(None);
    }

    #[test]
    fn unlock_local_vault_uses_legacy_raw_key_when_system_keyring_has_one() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let dir = temp_vault_dir("legacy-keyring");
        let path = dir.join(VAULT_FILE_NAME);
        let vault = LocalSecretVault::new(path.clone());
        let legacy_key = [77; 32];

        std::fs::create_dir_all(&dir).expect("vault dir");
        set_test_vault_dir_for_tests(Some(dir.clone()));
        unlock_local_vault_for_tests(legacy_key);
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");

        clear_cached_vault_key_for_tests();
        store_mock_legacy_system_keyring_key_for_tests(legacy_key);

        unlock_local_vault(None).expect("unlock");
        assert!(is_local_vault_unlocked());
        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));
        assert_eq!(1, mock_system_keyring_read_count_for_tests());

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
        set_test_vault_dir_for_tests(None);
    }

    #[test]
    fn unlock_local_vault_allows_passphrase_recovery_when_system_keyring_key_is_stale() {
        let _guard = TEST_GUARD.lock().expect("test guard");
        clear_cached_vault_key_for_tests();
        let dir = temp_vault_dir("stale-keyring");
        let path = dir.join(VAULT_FILE_NAME);
        let vault = LocalSecretVault::new(path.clone());
        let passphrase = "startup-secret";
        let correct_key = derive_master_key_from_passphrase(passphrase);

        std::fs::create_dir_all(&dir).expect("vault dir");
        set_test_vault_dir_for_tests(Some(dir.clone()));
        unlock_local_vault_for_tests(correct_key);
        vault
            .store("host_password", "host-a", "secret")
            .expect("store");

        clear_cached_vault_key_for_tests();
        store_mock_system_keyring_key_for_tests([99; 32]);

        let error = unlock_local_vault(None).unwrap_err();
        assert!(error.contains("系统钥匙串中的本地加密密钥无法解开现有 vault"));

        unlock_local_vault(Some(passphrase.to_string())).expect("recover unlock");
        assert_eq!("secret", vault.get("host_password", "host-a").expect("get"));

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
        set_test_vault_dir_for_tests(None);
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
