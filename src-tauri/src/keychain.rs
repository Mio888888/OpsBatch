use keyring::Entry;

const SERVICE_NAME: &str = "com.opsbatch.app";
const KEY_API_KEY: &str = "ai_api_key";
const KEY_HOST_PASSWORD: &str = "host_password";
const KEY_HOST_PRIVATE_KEY: &str = "host_private_key";
const KEY_GITHUB_TOKEN: &str = "github_token";
const KEY_SSH_HOST_KEY: &str = "ssh_host_key";

fn store_secret(kind: &str, id: &str, value: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}:{}", kind, id))
        .map_err(|e| format!("keyring 创建失败: {}", e))?;
    entry
        .set_password(value)
        .map_err(|e| format!("keyring 存储失败: {}", e))
}

fn get_secret(kind: &str, id: &str) -> Result<String, String> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}:{}", kind, id))
        .map_err(|e| format!("keyring 创建失败: {}", e))?;
    entry
        .get_password()
        .map_err(|e| format!("keyring 读取失败（可能需要 Touch ID 认证）: {}", e))
}

fn delete_secret(kind: &str, id: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}:{}", kind, id))
        .map_err(|e| format!("keyring 创建失败: {}", e))?;
    entry
        .delete_credential()
        .map_err(|e| format!("keyring 删除失败: {}", e))
}

pub fn store_api_key(provider: &str, api_key: &str) -> Result<(), String> {
    store_secret(KEY_API_KEY, provider, api_key)
}

pub fn get_api_key(provider: &str) -> Result<String, String> {
    get_secret(KEY_API_KEY, provider)
}

pub fn delete_api_key(provider: &str) -> Result<(), String> {
    delete_secret(KEY_API_KEY, provider)
}

pub fn store_host_password(host_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_HOST_PASSWORD, host_id, value)
}

pub fn get_host_password(host_id: &str) -> Result<String, String> {
    get_secret(KEY_HOST_PASSWORD, host_id)
}

pub fn delete_host_password(host_id: &str) -> Result<(), String> {
    delete_secret(KEY_HOST_PASSWORD, host_id)
}

pub fn store_host_private_key(host_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_HOST_PRIVATE_KEY, host_id, value)
}

pub fn get_host_private_key(host_id: &str) -> Result<String, String> {
    get_secret(KEY_HOST_PRIVATE_KEY, host_id)
}

pub fn delete_host_private_key(host_id: &str) -> Result<(), String> {
    delete_secret(KEY_HOST_PRIVATE_KEY, host_id)
}

pub fn store_github_token(repo_id: &str, value: &str) -> Result<(), String> {
    store_secret(KEY_GITHUB_TOKEN, repo_id, value)
}

pub fn get_github_token(repo_id: &str) -> Result<String, String> {
    get_secret(KEY_GITHUB_TOKEN, repo_id)
}

pub fn delete_github_token(repo_id: &str) -> Result<(), String> {
    delete_secret(KEY_GITHUB_TOKEN, repo_id)
}

pub fn store_ssh_host_key(host_id: &str, fingerprint: &str) -> Result<(), String> {
    store_secret(KEY_SSH_HOST_KEY, host_id, fingerprint)
}

pub fn get_ssh_host_key(host_id: &str) -> Result<String, String> {
    get_secret(KEY_SSH_HOST_KEY, host_id)
}
