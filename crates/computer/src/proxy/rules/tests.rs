use proptest::prelude::*;

use super::*;

fn host(text: &str) -> Host {
    Host::parse(text).unwrap()
}

fn rules(mode: NetworkMode, patterns: &[&str]) -> Rules {
    Rules::new(
        mode,
        patterns.iter().map(|p| HostPattern::parse(p).unwrap()),
    )
}

fn ips(texts: &[&str]) -> Vec<IpAddr> {
    texts.iter().map(|t| t.parse().unwrap()).collect()
}

#[test]
fn each_mode_decides_by_the_name_alone() {
    let off = rules(NetworkMode::Offline, &["example.com"]);
    assert_eq!(
        off.step(&host("example.com"), 443),
        Step::Refuse(Refusal::Off)
    );
    let open = rules(NetworkMode::Open, &[]);
    assert_eq!(open.step(&host("anything.example"), 5432), Step::Go);
    assert_eq!(
        open.step(&host("169.254.169.254"), 443),
        Step::Go,
        "checked after"
    );
    let guarded = rules(
        NetworkMode::Guarded,
        &[
            "registry.npmjs.org",
            "*.githubusercontent.com",
            "*.example.org:8443",
        ],
    );
    assert_eq!(guarded.step(&host("registry.npmjs.org"), 443), Step::Go);
    assert_eq!(
        guarded.step(&host("raw.githubusercontent.com"), 22),
        Step::Go
    );
    assert_eq!(guarded.step(&host("githubusercontent.com"), 443), Step::Ask);
    assert_eq!(guarded.step(&host("a.example.org"), 8443), Step::Go);
    assert_eq!(guarded.step(&host("a.example.org"), 443), Step::Ask);
    assert_eq!(guarded.step(&host("evil.example"), 443), Step::Ask);
    assert_eq!(guarded.step(&host("93.184.216.34"), 443), Step::Ask);
}

#[test]
fn only_public_addresses_are_kept() {
    assert_eq!(kept(&[]), Err(Refusal::NotFound));
    for withheld in [
        "169.254.169.254",
        "fd00:ec2::254",
        "::ffff:169.254.169.254",
        "10.0.0.5",
        "192.168.1.1",
        "127.0.0.1",
        "::1",
        "100.100.100.200",
        "64:ff9b::a9fe:a9fe",
    ] {
        assert_eq!(
            kept(&ips(&[withheld])),
            Err(Refusal::Withheld),
            "{withheld}"
        );
    }
    let mixed = ips(&["10.0.0.5", "93.184.216.34", "169.254.169.254"]);
    assert_eq!(kept(&mixed), Ok(ips(&["93.184.216.34"])));
}

#[test]
fn every_refusal_says_why_in_one_sentence() {
    for refusal in [
        Refusal::Off,
        Refusal::NotAllowed,
        Refusal::Withheld,
        Refusal::NotFound,
    ] {
        let said = refusal.to_string();
        assert!(said.ends_with('.') && !said.contains('\n'), "{said}");
    }
}

proptest! {
    #[test]
    fn no_v4_address_in_any_form_is_kept_unless_public(a: u8, b: u8, c: u8, d: u8) {
        let v4 = std::net::Ipv4Addr::new(a, b, c, d);
        let public = kept(&[IpAddr::V4(v4)]).is_ok();
        prop_assert_eq!(kept(&[IpAddr::V6(v4.to_ipv6_mapped())]).is_ok(), public);
        if glave_ports::host::is_inside(IpAddr::V4(v4)) || v4.is_loopback() {
            prop_assert!(!public);
        }
    }
}
