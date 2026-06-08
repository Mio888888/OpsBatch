use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

const KEY: &[u8; 32] = b"opsbatch-default-encryption-key!";

pub fn encrypt(plaintext: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(KEY).map_err(|e| e.to_string())?;
    let nonce_bytes = uuid::Uuid::new_v4();
    let nonce = Nonce::from_slice(&nonce_bytes.as_bytes()[..12]);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut result = nonce_bytes.as_bytes()[..12].to_vec();
    result.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&result))
}

pub fn decrypt(encoded: &str) -> Result<String, String> {
    let data = BASE64.decode(encoded).map_err(|e| e.to_string())?;
    if data.len() < 12 {
        return Err("Invalid encrypted data".to_string());
    }
    let cipher = Aes256Gcm::new_from_slice(KEY).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&data[..12]);
    let plaintext = cipher
        .decrypt(nonce, &data[12..])
        .map_err(|e| e.to_string())?;
    String::from_utf8(plaintext).map_err(|e| e.to_string())
}
