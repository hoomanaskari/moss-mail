use std::sync::Arc;

use rustls::pki_types::CertificateDer;
use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt as _;

use crate::{Error, Result};

/// Which certificates to trust.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TlsRoots {
    /// The operating system's trust store (Keychain on macOS), so company
    /// certificates the user installed work too.
    #[default]
    Platform,
    /// Only these DER-encoded certificates. For self-signed servers, and tests.
    Custom(Vec<Vec<u8>>),
}

pub(crate) fn client_config(roots: &TlsRoots) -> Result<Arc<ClientConfig>> {
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(tls_error)?;
    let config = match roots {
        TlsRoots::Platform => builder
            .with_platform_verifier()
            .map_err(tls_error)?
            .with_no_client_auth(),
        TlsRoots::Custom(certs) => {
            let mut store = rustls::RootCertStore::empty();
            for der in certs {
                store
                    .add(CertificateDer::from(der.clone()))
                    .map_err(tls_error)?;
            }
            builder.with_root_certificates(store).with_no_client_auth()
        }
    };
    Ok(Arc::new(config))
}

fn tls_error(e: rustls::Error) -> Error {
    Error::Network(format!("TLS setup failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_config_from_the_platform_store() {
        assert!(client_config(&TlsRoots::Platform).is_ok());
    }

    #[test]
    fn builds_a_config_from_custom_roots() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        assert!(client_config(&TlsRoots::Custom(vec![cert.cert.der().to_vec()])).is_ok());
    }

    #[test]
    fn refuses_a_root_that_isnt_a_certificate() {
        let e = client_config(&TlsRoots::Custom(vec![vec![1, 2, 3]])).unwrap_err();
        assert!(matches!(e, Error::Network(m) if m.starts_with("TLS setup failed")));
    }
}
