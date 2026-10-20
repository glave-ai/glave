//! HTTPS as browsers and the authority meet it: real TLS handshakes, on loopback, with what
//! `Https::tls` serves. Before a certificate is in hand no handshake completes; once one is
//! kept, it is served for its name alone, over HTTP/1.1, and the authority's protocol gets
//! nothing while no challenge is asked.

use std::net::SocketAddr;
use std::sync::Arc;

use glave_acme::Https;
use glave_event::{At, CertificateIssued, EventId, Header, Secret, Seq, encode};
use glave_ports::stream::Reserved;
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{EventLog as _, OrgId, ProjectId, Scope, block_on};
use glave_store_sqlite::{SqliteLog, Store};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use uuid::Uuid;

const NAME: &str = "glave.test";

fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    )
}

fn store() -> Store {
    let root = glave_keys::InMemoryRoot::new().unwrap();
    Store::open_in_memory(glave_keys::KeySettings::new(Arc::new(root))).unwrap()
}

async fn open(log: SqliteLog) -> Https<SqliteLog> {
    let clock = ManualClock::at_second(1_792_000_000);
    let directory = "https://acme.example.com/directory";
    let entropy = Box::new(CountingEntropy::starting_at(1));
    Https::open(log, scope(), NAME, directory, Box::new(clock), entropy)
        .await
        .unwrap()
}

/// An authority of the test's own, and a certificate it signed for `NAME`, kept in `log`'s
/// certificates ledger as glave keeps one it was issued: the authority's certificate, to
/// trust.
async fn keep_one(log: &SqliteLog) -> CertificateDer<'static> {
    let authority_key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority = params.self_signed(&authority_key).unwrap();
    let signer = rcgen::Issuer::from_params(&params, &authority_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec![NAME.to_owned()])
        .unwrap()
        .signed_by(&key, &signer)
        .unwrap();
    let at = At::from(jiff::Timestamp::from_second(1_792_000_000).unwrap());
    let issued = CertificateIssued {
        name: NAME.to_owned(),
        chain: certificate.pem(),
        key: Secret::new(key.serialize_pem()),
        not_before: at,
        not_after: at,
    };
    let header = Header {
        id: EventId::new(at.timestamp(), [3; 10]).unwrap(),
        thread: Reserved::Certificates.id(),
        seq: Seq::new(1).unwrap(),
        at,
    };
    log.append(&scope(), &[encode(header, &issued).unwrap()])
        .await
        .unwrap();
    authority.der().clone()
}

/// The protocol a client trusting `authority` agrees on with `https` for `name` when it offers
/// `protocols`, or `None` when the handshake fails.
async fn hello(
    https: &Https<SqliteLog>,
    authority: Option<&CertificateDer<'static>>,
    name: &str,
    protocols: &[&[u8]],
) -> Option<Vec<u8>> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let acceptor = TlsAcceptor::from(https.tls());
    let answering = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // A handshake that fails ends the connection, which the client sees.
        if let Ok(mut tls) = acceptor.accept(stream).await {
            let _ = tls.shutdown().await;
        }
    });
    let mut roots = RootCertStore::empty();
    if let Some(authority) = authority {
        roots.add(authority.clone()).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = protocols.iter().map(|protocol| protocol.to_vec()).collect();
    let stream = TcpStream::connect(address).await.unwrap();
    let server = ServerName::try_from(name.to_owned()).unwrap();
    let agreed = TlsConnector::from(Arc::new(config))
        .connect(server, stream)
        .await
        .ok()
        .map(|tls| tls.get_ref().1.alpn_protocol().unwrap_or_default().to_vec());
    answering.await.unwrap();
    agreed
}

#[test]
fn no_handshake_completes_before_a_certificate_and_one_kept_is_served_for_its_name_alone() {
    block_on!(async {
        let store = store();
        let fresh = open(store.log()).await;
        assert_eq!(hello(&fresh, None, NAME, &[b"http/1.1"]).await, None);

        let authority = keep_one(&store.log()).await;
        let https = open(store.log()).await;
        assert!(https.ready());
        let agreed = hello(&https, Some(&authority), NAME, &[b"h2", b"http/1.1"]).await;
        assert_eq!(agreed.as_deref(), Some(&b"http/1.1"[..]));
        let other = hello(&https, Some(&authority), "other.test", &[b"http/1.1"]).await;
        assert_eq!(other, None, "another name gets nothing");
        let asked = hello(&https, Some(&authority), NAME, &[b"acme-tls/1"]).await;
        assert_eq!(asked, None, "no challenge is asked, so none is answered");
    });
}
