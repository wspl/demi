//! The TLS client configuration, built on first use.
//!
//! The guest build (`guest-roots`) trusts the embedded webpki roots; every
//! other build uses the platform verifier, which is what makes corporate
//! MITM proxies work on user hosts. The conformance build can point
//! `DEMI_SHELL_CONFORMANCE_CA` at the stub's self-signed CA instead.

use std::sync::Arc;

use rustls::ClientConfig;

pub fn client_config() -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    #[cfg(feature = "conformance")]
    if let Some(roots) = conformance_roots() {
        return Ok(Arc::new(builder.with_root_certificates(roots).with_no_client_auth()));
    }
    #[cfg(feature = "guest-roots")]
    let config = {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    #[cfg(not(feature = "guest-roots"))]
    let config = {
        use rustls_platform_verifier::BuilderVerifierExt;
        builder.with_platform_verifier().map_err(|e| e.to_string())?.with_no_client_auth()
    };
    Ok(Arc::new(config))
}

#[cfg(feature = "conformance")]
fn conformance_roots() -> Option<rustls::RootCertStore> {
    use rustls::pki_types::pem::PemObject;
    let path = std::env::var("DEMI_SHELL_CONFORMANCE_CA").ok()?;
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(&path).ok()?;
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(certs.filter_map(|c| c.ok()));
    Some(roots)
}
