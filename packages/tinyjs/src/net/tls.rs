//! The TLS client configuration, built on first use.
//!
//! The guest build (`guest-roots`) trusts the embedded webpki roots; every
//! other build uses the platform verifier, which is what makes corporate
//! MITM proxies work on user hosts. `TINYJS_CA_FILE` replaces the root store
//! with the certificates in a PEM file, the way `SSL_CERT_FILE` does for
//! OpenSSL; the conformance suite uses it for its self-signed stub.

use std::sync::Arc;

use rustls::ClientConfig;

pub fn client_config() -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    if let Some(roots) = ca_file_roots() {
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

fn ca_file_roots() -> Option<rustls::RootCertStore> {
    use rustls::pki_types::pem::PemObject;
    let path = std::env::var("TINYJS_CA_FILE").ok()?;
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(&path).ok()?;
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(certs.filter_map(|c| c.ok()));
    Some(roots)
}
