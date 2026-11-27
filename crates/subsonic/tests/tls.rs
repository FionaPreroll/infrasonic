// SPDX-License-Identifier: GPL-3.0-only
//! A server with a self-signed certificate (`testdata/tls`): trusted by its fingerprint, on
//! first use, or not at all.

use std::sync::Arc;

use infrasonic_subsonic::{Auth, Client, certificate_fingerprint};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/tls");

fn cert(name: &str) -> CertificateDer<'static> {
    CertificateDer::from_pem_file(format!("{TLS}/{name}.crt")).expect("certificate")
}

/// An HTTPS server on localhost that answers every request with a successful `ping`.
async fn server(name: &str) -> String {
    infrasonic_subsonic::ensure_crypto_provider();
    let key = PrivateKeyDer::from_pem_file(format!("{TLS}/{name}.key")).expect("key");
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert(name)], key)
        .expect("server config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = vec![0u8; 4096];
                let _ = tls.read(&mut buf).await;
                let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(resp.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    format!("https://localhost:{}", addr.port())
}

fn client(url: &str, pin: Option<Option<&str>>) -> Client {
    let b = Client::builder(url, Auth::ApiKey { key: "k".into() });
    match pin {
        Some(p) => b.trust_own_certificate(p),
        None => b,
    }
    .build()
    .expect("client")
}

#[tokio::test]
async fn own_certificate_is_trusted_on_first_use_then_pinned() {
    let url = server("server").await;
    let fp = certificate_fingerprint(cert("server").as_ref());

    // Not without being asked.
    let plain = client(&url, None);
    assert!(plain.ping().await.is_err(), "self-signed accepted");
    assert_eq!(plain.certificate(), None);

    // First use: any certificate, and the client says which.
    let first = client(&url, Some(None));
    assert_eq!(first.certificate(), None);
    first.ping().await.expect("first use");
    assert_eq!(first.certificate().as_deref(), Some(fp.as_str()));

    // Pinned, also written with colons and in capitals.
    let colons = fp
        .as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_uppercase())
        .collect::<Vec<_>>()
        .join(":");
    client(&url, Some(Some(&colons)))
        .ping()
        .await
        .expect("pinned");

    // Another certificate is refused, and the message says why.
    let other = certificate_fingerprint(cert("other").as_ref());
    let pinned_other = client(&url, Some(Some(&other)));
    let e = pinned_other.ping().await.expect_err("wrong certificate");
    let chain = std::iter::successors(Some(&e as &dyn std::error::Error), |e| e.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    assert!(chain.contains("certificate changed"), "{chain}");
    assert!(
        !chain.contains("apiKey"),
        "login data in the error: {chain}"
    );
    assert_eq!(pinned_other.certificate().as_deref(), Some(fp.as_str()));

    // A malformed fingerprint trusts nothing.
    assert!(client(&url, Some(Some("nonsense"))).ping().await.is_err());
}
