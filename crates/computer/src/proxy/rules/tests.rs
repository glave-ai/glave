use proptest::prelude::*;

use super::*;

fn host(text: &str) -> Host {
    Host::parse(text).unwrap()
}

fn allowed(patterns: &[&str]) -> Rules {
    Rules::new(
        Mode::Limited,
        patterns.iter().map(|p| Allowed::parse(p).unwrap()),
    )
}

fn ips(texts: &[&str]) -> Vec<IpAddr> {
    texts.iter().map(|t| t.parse().unwrap()).collect()
}

#[test]
fn hosts_are_names_or_addresses() {
    assert_eq!(host("Example.COM."), Host::Name("example.com".into()));
    assert_eq!(
        host("93.184.216.34"),
        Host::Address(ips(&["93.184.216.34"])[0])
    );
    assert_eq!(host("[::1]"), Host::Address(ips(&["::1"])[0]));
    for bad in [
        "",
        ".",
        "a..b",
        "a b",
        "::1",
        "[nope]",
        "127.1",
        "2130706433",
        "1.2.3.4.",
    ] {
        assert_eq!(Host::parse(bad), None, "{bad}");
    }
    assert_eq!(Host::parse(&"a".repeat(64)), None);
    assert_eq!(host("[2606:4700::1111]").to_string(), "[2606:4700::1111]");
}

#[test]
fn a_pattern_is_a_name_with_or_without_a_star_before_it() {
    assert!(Allowed::parse("*.example.com").is_some());
    assert!(Allowed::parse("example.com").is_some());
    for bad in ["*", "*.", "a.*.com", "**.com", "*example.com", "1.2.3.4"] {
        assert_eq!(Allowed::parse(bad), None, "{bad}");
    }
}

#[test]
fn each_mode_decides_by_the_name_alone() {
    let off = Rules::new(Mode::None, Allowed::parse("example.com"));
    assert_eq!(off.step(&host("example.com")), Step::Refuse(Refusal::Off));
    let full = Rules::new(Mode::Full, None);
    assert_eq!(full.step(&host("anything.example")), Step::Go);
    assert_eq!(
        full.step(&host("169.254.169.254")),
        Step::Go,
        "checked after"
    );
    let limited = allowed(&["registry.npmjs.org", "*.githubusercontent.com"]);
    assert_eq!(limited.step(&host("registry.npmjs.org")), Step::Go);
    assert_eq!(limited.step(&host("raw.githubusercontent.com")), Step::Go);
    assert_eq!(limited.step(&host("githubusercontent.com")), Step::Ask);
    assert_eq!(limited.step(&host("evil.example")), Step::Ask);
    assert_eq!(limited.step(&host("93.184.216.34")), Step::Ask);
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

fn label() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9-]{0,10}"
}

fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(label(), 1..4).prop_map(|labels| format!("{}.test", labels.join(".")))
}

proptest! {
    #[test]
    fn a_star_admits_every_name_under_it_and_never_itself(base in name(), sub in label()) {
        let rules = allowed(&[&format!("*.{base}")]);
        prop_assert_eq!(rules.step(&host(&format!("{sub}.{base}"))), Step::Go);
        prop_assert_eq!(rules.step(&host(&base)), Step::Ask);
        // A name that only ends with the same letters is another host.
        prop_assert_eq!(rules.step(&host(&format!("{sub}{base}"))), Step::Ask);
    }

    #[test]
    fn an_exact_name_admits_itself_alone_in_any_case(base in name(), sub in label()) {
        let rules = allowed(&[&base]);
        prop_assert_eq!(rules.step(&host(&base.to_ascii_uppercase())), Step::Go);
        prop_assert_eq!(rules.step(&host(&format!("{base}."))), Step::Go);
        prop_assert_eq!(rules.step(&host(&format!("{sub}.{base}"))), Step::Ask);
    }

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
