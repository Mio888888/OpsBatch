//! RFB authentication primitives: classic VNC (DES challenge) and Apple Remote Desktop (type 30, DH + AES-128-ECB).

#[cfg(test)]
use aes::cipher::BlockDecrypt;
use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use anyhow::{bail, Result};
use md5::{Digest, Md5};
use num_bigint::BigUint;
use rand::RngCore;

/// RFB VNC authentication: DES-ECB over the 16-byte challenge, key = password (8 bytes, zero padded)
/// with the bits of every key byte reversed.
pub fn vnc_auth_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (i, b) in password.bytes().take(8).enumerate() {
        key[i] = b.reverse_bits();
    }
    let cipher = des::Des::new(GenericArray::from_slice(&key));
    let mut out = *challenge;
    for chunk in out.chunks_exact_mut(8) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    out
}

/// Data the server sends for security type 30 (ARD).
pub struct ArdServerHello {
    pub generator: u16,
    pub prime: Vec<u8>,
    pub server_public: Vec<u8>,
}

fn pad_be(value: &BigUint, len: usize) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let mut out = vec![0u8; len.saturating_sub(bytes.len())];
    out.extend_from_slice(&bytes);
    out
}

fn fill_field(dst: &mut [u8], text: &str) {
    // Fixed 64-byte field: NUL-terminated text, remainder random (as noVNC / Apple clients do).
    rand::thread_rng().fill_bytes(dst);
    let bytes = text.as_bytes();
    let n = bytes.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&bytes[..n]);
    dst[n] = 0;
}

/// Client side of ARD authentication. Returns `(encrypted_credentials[128], client_public[keylen])`.
pub fn ard_client_response(
    hello: &ArdServerHello,
    username: &str,
    password: &str,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let key_len = hello.prime.len();
    if key_len == 0 || key_len != hello.server_public.len() || key_len > 1024 {
        bail!("invalid ARD key length {key_len}");
    }
    let p = BigUint::from_bytes_be(&hello.prime);
    let g = BigUint::from(hello.generator);
    let server_pub = BigUint::from_bytes_be(&hello.server_public);

    let mut priv_bytes = vec![0u8; key_len];
    rand::thread_rng().fill_bytes(&mut priv_bytes);
    let private = BigUint::from_bytes_be(&priv_bytes);

    let client_public = pad_be(&g.modpow(&private, &p), key_len);
    let shared = pad_be(&server_pub.modpow(&private, &p), key_len);
    let aes_key: [u8; 16] = Md5::digest(&shared).into();

    let mut creds = [0u8; 128];
    fill_field(&mut creds[..64], username);
    fill_field(&mut creds[64..], password);
    let cipher = aes::Aes128::new(GenericArray::from_slice(&aes_key));
    for block in creds.chunks_exact_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }
    Ok((creds.to_vec(), client_public))
}

/// Server side of ARD (used by the mock server in tests): decrypt and split the credentials.
#[cfg(test)]
pub fn ard_server_decrypt(
    prime: &[u8],
    private: &BigUint,
    client_public: &[u8],
    encrypted: &[u8],
) -> (String, String) {
    let p = BigUint::from_bytes_be(prime);
    let shared = pad_be(
        &BigUint::from_bytes_be(client_public).modpow(private, &p),
        prime.len(),
    );
    let aes_key: [u8; 16] = Md5::digest(&shared).into();
    let cipher = aes::Aes128::new(GenericArray::from_slice(&aes_key));
    let mut buf = encrypted.to_vec();
    for block in buf.chunks_exact_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    let cstr = |b: &[u8]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    };
    (cstr(&buf[..64]), cstr(&buf[64..128]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnc_key_bits_are_reversed() {
        assert_eq!(0b0000_0001u8.reverse_bits(), 0b1000_0000);
        // Server-side check mirrors the client; also guards against accidental no-op.
        let challenge = [7u8; 16];
        let a = vnc_auth_response("secret", &challenge);
        let b = vnc_auth_response("secret", &challenge);
        let c = vnc_auth_response("Secret", &challenge);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, challenge);
    }

    #[test]
    fn vnc_password_longer_than_8_is_truncated() {
        let ch = [3u8; 16];
        assert_eq!(
            vnc_auth_response("12345678", &ch),
            vnc_auth_response("12345678XYZ", &ch)
        );
    }

    #[test]
    fn ard_roundtrip() {
        // 128-byte (1024-bit) odd prime for the test; real servers send a 128-byte DH prime.
        let prime = hex(
            "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece65381ffffffffffffffff",
        );
        let g = 2u16;
        let server_priv = BigUint::from_bytes_be(&[0x42; 128]);
        let server_pub = pad_be(
            &BigUint::from(g).modpow(&server_priv, &BigUint::from_bytes_be(&prime)),
            128,
        );
        let hello = ArdServerHello {
            generator: g,
            prime: prime.clone(),
            server_public: server_pub,
        };
        let (enc, client_pub) = ard_client_response(&hello, "alice", "p@ssw0rd").unwrap();
        assert_eq!(enc.len(), 128);
        assert_eq!(client_pub.len(), 128);
        let (u, p) = ard_server_decrypt(&prime, &server_priv, &client_pub, &enc);
        assert_eq!((u.as_str(), p.as_str()), ("alice", "p@ssw0rd"));
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
