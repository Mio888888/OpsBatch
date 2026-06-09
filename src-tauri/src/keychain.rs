#[cfg(not(target_os = "macos"))]
use keyring::{Entry, Error as KeyringError};
#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
#[cfg(target_os = "macos")]
use security_framework::os::macos::passwords::find_generic_password;

const SERVICE_NAME: &str = "com.opsbatch.app";
const KEY_API_KEY: &str = "ai_api_key";
const KEY_HOST_PASSWORD: &str = "host_password";
const KEY_HOST_PRIVATE_KEY: &str = "host_private_key";
const KEY_GITHUB_TOKEN: &str = "github_token";
const KEY_SSH_HOST_KEY: &str = "ssh_host_key";

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

fn store_secret(kind: &str, id: &str, value: &str) -> Result<(), String> {
    let account = secret_account(kind, id);
    #[cfg(target_os = "macos")]
    {
        let keychain = macos_keychain()?;
        keychain
            .set_generic_password(SERVICE_NAME, &account, value.as_bytes())
            .map_err(|e| format!("macOS keychain 存储失败: {}", e))?;
        match macos_get_secret(&account) {
            Ok(stored) if stored == value => return Ok(()),
            Ok(_) => return Err("macOS keychain 写入后校验失败：读回内容不匹配".to_string()),
            Err(error) => {
                return Err(format!(
                    "macOS keychain 写入后校验失败 service={} account={} error={}",
                    SERVICE_NAME, account, error
                ));
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let entry =
            Entry::new(SERVICE_NAME, &account).map_err(|e| format!("keyring 创建失败: {}", e))?;
        entry
            .set_password(value)
            .map_err(|e| format!("keyring 存储失败: {}", e))?;
        let verify_entry = Entry::new(SERVICE_NAME, &account)
            .map_err(|e| format!("keyring 校验 entry 创建失败: {}", e))?;
        match verify_entry.get_password() {
            Ok(stored) if stored == value => Ok(()),
            Ok(_) => Err("keyring 写入后校验失败：读回内容不匹配".to_string()),
            Err(e) => Err(format!(
                "keyring 写入后校验失败 service={} account={} error={}",
                SERVICE_NAME, account, e
            )),
        }
    }
}

fn get_secret(kind: &str, id: &str) -> Result<String, SecretError> {
    let account = secret_account(kind, id);
    #[cfg(target_os = "macos")]
    {
        return macos_get_secret(&account);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let entry = Entry::new(SERVICE_NAME, &account)
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

fn delete_secret(kind: &str, id: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let account = secret_account(kind, id);
        match find_generic_password(Some(&[macos_keychain()?]), SERVICE_NAME, &account) {
            Ok((_, item)) => {
                item.delete();
                return Ok(());
            }
            Err(error) => return Err(format!("macOS keychain 删除失败: {}", error)),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let entry = Entry::new(SERVICE_NAME, &secret_account(kind, id))
            .map_err(|e| format!("keyring 创建失败: {}", e))?;
        entry
            .delete_credential()
            .map_err(|e| format!("keyring 删除失败: {}", e))
    }
}

fn secret_account(kind: &str, id: &str) -> String {
    format!("{}:{}", kind, id)
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
