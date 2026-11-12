use proptest::prelude::*;

use super::*;

fn pattern(text: &str) -> HostPattern {
    HostPattern::parse(text).unwrap()
}

#[test]
fn a_pattern_is_a_host_with_or_without_a_star_before_it_and_a_port_after_it() {
    for (text, written) in [
        ("*.Example.com.", "*.example.com"),
        (" example.com ", "example.com"),
        ("*.example.com:8443", "*.example.com:8443"),
        ("*", "*"),
        ("*:22", "*:22"),
        ("127.0.0.1", "127.0.0.1"),
        ("::1", "[::1]"),
        ("[::1]:8080", "[::1]:8080"),
    ] {
        assert_eq!(pattern(text).to_string(), written, "{text}");
    }
    for bad in [
        "",
        "*.",
        "a.*.com",
        "**.com",
        "*example.com",
        "*.1.2.3",
        "127.0.0.*",
        "example.com:0",
        "example.com:x",
        "example.com:",
        "https://docs.rs",
        "docs.rs/path",
        "dö.cs",
    ] {
        assert!(HostPattern::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn addresses_are_named_exactly_however_written() {
    let local = pattern("127.0.0.1");
    assert!(local.is_exact() && local.names("127.0.0.1"));
    assert!(local.names("[::ffff:127.0.0.1]"));
    assert!(!local.names("localhost") && !local.names("127.0.0.2"));
    assert!(pattern("[::1]").names("[::1]") && pattern("::1").names("::1"));
    assert!(!pattern("*.example.com").is_exact() && !pattern("*").is_exact());
    assert!(pattern("*").names("[::1]") && pattern("*").names("anything.at.all"));
}

fn label() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9_-]{0,10}"
}

fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(label(), 1..4).prop_map(|labels| format!("{}.test", labels.join(".")))
}

proptest! {
    #[test]
    fn a_star_names_every_name_under_it_and_never_itself(base in name(), sub in label()) {
        let under = pattern(&format!("*.{base}"));
        let (one, two) = (format!("{sub}.{base}"), format!("{sub}.{sub}.{base}"));
        // A name that only ends with the same letters is another host.
        let alike = format!("{sub}{base}");
        prop_assert!(under.names(&one) && under.names(&two));
        prop_assert!(!under.names(&base) && !under.names(&alike));
    }

    #[test]
    fn an_exact_name_names_itself_alone_in_any_case(base in name(), sub in label()) {
        let loud = base.to_ascii_uppercase();
        let exact = pattern(&loud);
        let dotted = format!("{loud}.");
        let (under, over) = (format!("{sub}.{base}"), format!("{base}.{sub}"));
        prop_assert!(exact.names(&base) && exact.names(&dotted));
        prop_assert!(!exact.names(&under) && !exact.names(&over));
    }

    #[test]
    fn a_pattern_with_a_port_admits_that_port_alone_and_one_without_any(
        base in name(),
        own in 1..=u16::MAX,
        other in 1..=u16::MAX,
    ) {
        let any = pattern(&base);
        let ported = pattern(&format!("{base}:{own}"));
        prop_assert!(any.admits(&base, other) && ported.admits(&base, own));
        prop_assert_eq!(ported.admits(&base, other), own == other);
        prop_assert!(ported.names(&base));
    }

    #[test]
    fn a_pattern_reads_back_as_it_is_written(base in name(), port in 1..=u16::MAX) {
        let texts = [
            base.clone(),
            format!("*.{base}"),
            format!("*.{base}:{port}"),
            format!("{base}:{port}"),
        ];
        for text in texts {
            let read = pattern(&text);
            prop_assert_eq!(pattern(&read.to_string()), read.clone());
            prop_assert_eq!(read.to_string(), text);
        }
    }
}
