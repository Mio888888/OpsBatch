pub fn install_default_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_rustls_crypto_provider_for_ambiguous_feature_sets() {
        install_default_crypto_provider();

        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        install_default_crypto_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
