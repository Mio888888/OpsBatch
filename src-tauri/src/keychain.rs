use keyring::Entry;

const SERVICE_NAME: &str = "com.opsbatch.app";
const KEY_API_KEY: &str = "ai_api_key";

pub fn store_api_key(provider: &str, api_key: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}:{}", KEY_API_KEY, provider))
        .map_err(|e| format!("keyring 创建失败: {}", e))?;
    entry
        .set_password(api_key)
        .map_err(|e| format!("keyring 存储失败: {}", e))
}

pub fn get_api_key(provider: &str) -> Result<String, String> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}:{}", KEY_API_KEY, provider))
        .map_err(|e| format!("keyring 创建失败: {}", e))?;
    entry
        .get_password()
        .map_err(|e| format!("keyring 读取失败（可能需要 Touch ID 认证）: {}", e))
}

pub fn delete_api_key(provider: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &format!("{}:{}", KEY_API_KEY, provider))
        .map_err(|e| format!("keyring 创建失败: {}", e))?;
    entry
        .delete_credential()
        .map_err(|e| format!("keyring 删除失败: {}", e))
}
