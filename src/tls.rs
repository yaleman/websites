use std::path::PathBuf;

use anyhow::{Context, anyhow};
use axum_server::tls_rustls::RustlsConfig;

pub fn install_rustls_crypto_provider() -> anyhow::Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow!("failed to install rustls crypto provider"))?;

    Ok(())
}

pub async fn build_tls_config(
    tls_cert_path: &PathBuf,
    tls_key_path: &PathBuf,
) -> anyhow::Result<RustlsConfig> {
    install_rustls_crypto_provider()?;

    RustlsConfig::from_pem_file(tls_cert_path, tls_key_path)
        .await
        .with_context(|| {
            format!(
                "failed to load TLS PEM certificate/key from {} and {}",
                tls_cert_path.display(),
                tls_key_path.display()
            )
        })
}
