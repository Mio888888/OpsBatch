#[derive(Clone)]
pub struct HostKeyVerifier {
    host_id: String,
}

impl HostKeyVerifier {
    pub fn new(host_id: impl Into<String>) -> Self {
        Self {
            host_id: host_id.into(),
        }
    }

    pub fn verify_fingerprint(&self, fingerprint: &str) -> Result<bool, String> {
        match crate::keychain::get_ssh_host_key(&self.host_id) {
            Ok(stored) if stored == fingerprint => Ok(true),
            Ok(stored) => Err(format!(
                "SSH 主机密钥指纹不匹配：已保存 {}，当前 {}。如确认主机已更换密钥，请删除后重新添加主机。",
                stored, fingerprint
            )),
            Err(crate::keychain::SecretError::Missing) => {
                crate::keychain::store_ssh_host_key(&self.host_id, fingerprint)?;
                Ok(true)
            }
            Err(error) => Err(error.to_string()),
        }
    }
}
