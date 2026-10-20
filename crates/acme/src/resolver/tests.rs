use super::*;

/// A self-signed certificate for `name` and its key, as PEM.
fn self_signed(name: &str) -> (String, String) {
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec![name.to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    (certificate.pem(), key.serialize_pem())
}

#[test]
fn a_certificate_and_its_key_as_pem_are_served_and_anything_else_is_refused() {
    let (chain, key) = self_signed("glave.test");
    assert_eq!(certified(&chain, &key).unwrap().cert.len(), 1);
    for (chain, key) in [
        ("not pem", key.as_str()),
        ("", key.as_str()),
        (chain.as_str(), "not a key"),
    ] {
        assert!(certified(chain, key).is_err(), "{chain:?}");
    }
}

#[test]
fn the_challenge_s_certificate_is_for_the_name_and_carries_the_digest() {
    let digest = [7u8; 32];
    let key = challenge_key("glave.test", &digest).unwrap();
    let der = key.cert.first().unwrap();
    let (_, parsed) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
    // id-pe-acmeIdentifier, critical, its value an OCTET STRING of the digest (RFC 8737).
    let acme = parsed
        .extensions()
        .iter()
        .find(|extension| extension.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
        .unwrap();
    assert!(acme.critical);
    assert_eq!(acme.value, [&[0x04, 0x20][..], &digest].concat());
    let alternative = parsed.subject_alternative_name().unwrap().unwrap();
    let shown = format!("{:?}", alternative.value.general_names);
    assert!(shown.contains("glave.test"), "{shown}");
}

#[test]
fn a_resolver_is_ready_once_it_serves_a_certificate_and_never_shows_a_key() {
    let resolver = Resolver::new("glave.test".into());
    assert!(!resolver.ready());
    let (chain, key) = self_signed("glave.test");
    resolver.serve(certified(&chain, &key).unwrap());
    assert!(resolver.ready());
    let shown = format!("{resolver:?}");
    assert!(
        shown.contains("ready: true") && !shown.contains("PRIVATE"),
        "{shown}"
    );
}
