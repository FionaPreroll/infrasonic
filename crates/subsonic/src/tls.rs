// SPDX-License-Identifier: GPL-3.0-only
//! Servers with a certificate of their own (self-signed, a home CA): instead of trusting any
//! certificate, the client trusts exactly one, named by the SHA-256 of its DER encoding. Without
//! a fingerprint it takes the one the server presents and reports it, so that the caller can
//! keep it for the next connections (trust on first use).

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// SHA-256 of a certificate as lowercase hex.
pub fn fingerprint(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

/// `AB:CD:…` or `abcd…` → `abcd…`; `None` if it is no SHA-256 fingerprint.
pub fn normalize_fingerprint(s: &str) -> Option<String> {
    let hex: String = s
        .chars()
        .filter(|c| !matches!(c, ':' | ' '))
        .collect::<String>()
        .to_ascii_lowercase();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// Accepts the server's certificate if its fingerprint is the pinned one (or any, without a
/// pin) and remembers the fingerprint it saw. The handshake signatures are still checked, so
/// only the holder of the certificate's key gets through.
#[derive(Debug)]
pub(crate) struct PinVerifier {
    pin: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinVerifier {
    pub(crate) fn new(pin: Option<String>, seen: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            pin,
            seen,
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        if let Ok(mut s) = self.seen.lock() {
            *s = Some(fp.clone());
        }
        match &self.pin {
            Some(pin) if *pin != fp => Err(rustls::Error::General(format!(
                "the server's certificate changed (SHA-256 {}, trusted was {})",
                short(&fp),
                short(pin)
            ))),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// The first 16 hex digits, enough to tell certificates apart in a message.
fn short(fp: &str) -> &str {
    fp.get(..16).unwrap_or(fp)
}

/// A rustls configuration that trusts only through `verifier`.
pub(crate) fn config(verifier: PinVerifier) -> Result<rustls::ClientConfig, rustls::Error> {
    let mut c = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(verifier))
    .with_no_client_auth();
    c.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_normalised() {
        let fp = fingerprint(b"cert");
        assert_eq!(fp.len(), 64);
        let colons = fp
            .as_bytes()
            .chunks(2)
            .map(|c| String::from_utf8_lossy(c).to_uppercase())
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(normalize_fingerprint(&colons).as_deref(), Some(fp.as_str()));
        assert_eq!(normalize_fingerprint("abc"), None);
        assert_eq!(normalize_fingerprint(&"g".repeat(64)), None);
    }

    #[test]
    fn only_the_pinned_certificate_passes() {
        let seen = Arc::new(Mutex::new(None));
        let cert = CertificateDer::from(b"cert".to_vec());
        let other = CertificateDer::from(b"other".to_vec());
        let name = ServerName::try_from("music.local").expect("name");
        let v = PinVerifier::new(Some(fingerprint(b"cert")), seen.clone());
        assert!(
            v.verify_server_cert(&cert, &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        let e = v
            .verify_server_cert(&other, &[], &name, &[], UnixTime::now())
            .expect_err("other certificate");
        assert!(e.to_string().contains("changed"), "{e}");
        assert_eq!(
            seen.lock().expect("lock").as_deref(),
            Some(fingerprint(b"other").as_str())
        );
        // Without a pin: any certificate, and the caller learns which.
        let first = PinVerifier::new(None, seen.clone());
        assert!(
            first
                .verify_server_cert(&cert, &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        assert_eq!(
            seen.lock().expect("lock").as_deref(),
            Some(fingerprint(b"cert").as_str())
        );
    }
}
