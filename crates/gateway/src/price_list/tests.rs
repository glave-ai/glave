use std::fmt::Write as _;
use std::sync::Arc;

use glave_event::TokenCounts;
use glave_ports::testing::ScriptedModel;

use super::*;

/// One entry as the list writes it, with `fields` between its model and its date.
fn entry(service: &str, model: &str, fields: &str) -> String {
    format!(
        "[[prices]]\nservice = \"{service}\"\nmodel = \"{model}\"\n{fields}\n\
         date = \"2026-11-20\"\npage = \"https://example.com/pricing\"\n"
    )
}

/// Dollars as decimal text, from `nanos` billionths.
fn dollars(nanos: i64) -> String {
    format!("{}.{:09}", nanos / 1_000_000_000, nanos % 1_000_000_000)
}

/// Every price as one line: what the list says, in the order it keeps.
fn lines(list: &PriceList) -> String {
    let mut out = format!("{}\n", list.name);
    for ((service, model), entry) in &list.entries {
        let price = list.price(service, model).unwrap();
        let _ = writeln!(
            out,
            "{service} {model}: in {} out {} read {} write {} long {:?} images {:?} \
             request {} ({}, {})",
            price.input_per_mtok.decimal(),
            price.output_per_mtok.decimal(),
            price.cache_read_per_mtok.decimal(),
            price.cache_write_per_mtok.decimal(),
            price.long.map(|long| {
                let at = [
                    long.input_per_mtok,
                    long.output_per_mtok,
                    long.cache_read_per_mtok,
                    long.cache_write_per_mtok,
                ];
                (long.above, at.map(|each| each.decimal().to_string()))
            }),
            price
                .image_output_per_mtok
                .map(|each| each.decimal().to_string()),
            price.per_request.decimal(),
            entry.date,
            entry.page,
        );
    }
    out
}

#[test]
fn the_built_in_list_reads_and_says_every_price() {
    let list = PriceList::built_in().unwrap();
    insta::assert_snapshot!(lines(&list));
}

#[test]
fn every_entry_cites_its_page_and_its_date() {
    let priced = "input = \"1\"\noutput = \"2\"";
    let without = |line: &str| entry("s", "m", priced).replace(line, "");
    assert!(PriceList::read(&entry("s", "m", priced)).is_ok());
    for line in [
        "date = \"2026-11-20\"",
        "page = \"https://example.com/pricing\"",
    ] {
        assert!(
            PriceList::read(&without(line)).is_err(),
            "an entry without {line} is refused"
        );
    }
    let plain = entry("s", "m", priced).replace("https://example.com/pricing", "example.com");
    assert!(matches!(
        PriceList::read(&plain),
        Err(PriceListError::Unpriced(_))
    ));
}

#[test]
fn a_model_is_priced_once_at_a_service() {
    let priced = "input = \"1\"\noutput = \"2\"";
    let twice = format!("{}\n{}", entry("s", "m", priced), entry("s", "m", priced));
    assert!(matches!(
        PriceList::read(&twice),
        Err(PriceListError::Unpriced(error)) if error.reason == "is written twice"
    ));
    let apart = format!("{}\n{}", entry("s", "m", priced), entry("t", "m", priced));
    assert_eq!(PriceList::read(&apart).unwrap().entries.len(), 2);
}

#[test]
fn a_model_is_found_by_its_id_or_its_undated_id_and_only_at_its_service() {
    let list = PriceList::built_in().unwrap();
    let dated = list
        .price("anthropic", "claude-haiku-4-5-20251001")
        .unwrap();
    assert_eq!(dated.list, list.name);
    assert_eq!(Some(dated), list.price("anthropic", "claude-haiku-4-5"));
    assert!(list.price("openrouter", "claude-haiku-4-5").is_none());
    assert!(list.price("anthropic", "claude-unknown-1").is_none());
    assert!(list.price("anthropic", "claude-unknown-20251001").is_none());
    let snapshot = list.price("openai", "gpt-5.2-2025-12-11");
    assert!(snapshot.is_some(), "OpenAI's dated snapshot");
    assert_eq!(snapshot, list.price("openai", "gpt-5.2"));
    for not_a_date in [
        "gpt-5.2-25-12-11",
        "gpt-5.2-2025-1-11",
        "gpt-5.2-2025-12-1x",
        "2025-12-11",
    ] {
        assert!(list.price("openai", not_a_date).is_none(), "{not_a_date}");
    }
}

#[test]
fn the_list_wins_over_a_catalog_and_leaves_what_it_does_not_name() {
    let list = PriceList::built_in().unwrap();
    let handle = Arc::new(ScriptedModel::new(Vec::new()));
    let catalog = Price::per_mtok(
        "openrouter catalog",
        Nanos::new(1),
        Nanos::new(1),
        Nanos::new(1),
    );
    let at = |provider: &str, model: &str| {
        Target::new(
            Arc::clone(&handle) as Arc<dyn crate::Model>,
            provider,
            model,
        )
        .with_price(catalog.clone())
    };
    let direct = list.priced(at("anthropic", "claude-haiku-5-5"));
    assert_eq!(
        direct.price.as_ref().map(|p| p.list.as_str()),
        Some(list.name.as_str())
    );
    let routed = list.priced(at("openrouter", "anthropic/claude-haiku-5.5"));
    assert_eq!(
        routed.price,
        Some(catalog),
        "a router keeps its catalog's price"
    );
    let unknown = Target::new(handle, "anthropic", "claude-unknown-1");
    assert!(list.priced(unknown).price.is_none(), "no price is made up");
}

/// The payer's own prices, as their settings write them.
fn payer(text: &str) -> Vec<PriceEntry> {
    toml::from_str::<File>(text).unwrap().prices
}

#[test]
fn the_payer_s_prices_win_over_the_list_and_add_what_it_lacks() {
    let own = [
        entry(
            "anthropic",
            "claude-haiku-5-5",
            "input = \"0.01\"\noutput = \"0.02\"",
        ),
        entry(
            "hope.example.com",
            "hope",
            "input = \"0.03\"\noutput = \"0.04\"",
        ),
    ]
    .join("\n");
    let list = PriceList::built_in().unwrap();
    let under = list.clone().under(payer(&own)).unwrap();
    let haiku = under.price("anthropic", "claude-haiku-5-5").unwrap();
    assert_eq!(haiku.list, "the server's settings");
    assert_eq!(haiku.input_per_mtok, Nanos::new(10_000_000));
    let glave = list.price("anthropic", "claude-haiku-5-5").unwrap();
    assert_eq!(glave.list, list.name, "the list alone, its own");
    let hope = under.price("hope.example.com", "hope").unwrap();
    assert_eq!(
        hope.output_per_mtok,
        Nanos::new(40_000_000),
        "one no list names"
    );
    assert!(list.price("hope.example.com", "hope").is_none());
    let opus = under.price("anthropic", "claude-opus-5-5");
    assert!(opus.is_some());
    assert_eq!(
        opus,
        list.price("anthropic", "claude-opus-5-5"),
        "the rest, the list's"
    );
    assert_eq!(list.clone().under(Vec::new()).unwrap(), list);
}

#[test]
fn the_payer_s_prices_are_checked_as_the_list_s_are() {
    let list = PriceList::built_in().unwrap();
    let twice = entry("s", "m", "input = \"1\"\noutput = \"2\"").repeat(2);
    let unpriced = entry("s", "m", "input = \"1\"");
    for (text, reason) in [
        (twice, "is written twice"),
        (unpriced, "has no price for input and for what it makes"),
    ] {
        let refused = list.clone().under(payer(&text));
        assert!(
            matches!(&refused, Err(PriceListError::Unpriced(error)) if error.reason == reason),
            "{refused:?}"
        );
    }
}

#[test]
fn a_long_prompt_and_a_price_per_request_are_charged_as_the_list_says() {
    let text = [
        entry(
            "anthropic",
            "haiku",
            "input = \"0.10\"\noutput = \"0.50\"\n\
             long = { above = 100000, input = \"0.50\", output = \"2.50\" }",
        ),
        entry("api.search.brave.com", "search", "per_request = \"0.005\""),
    ]
    .join("\n");
    let list = PriceList::read(&text).unwrap();
    let haiku = list.price("anthropic", "haiku").unwrap();
    let counts = |input| TokenCounts {
        input,
        ..TokenCounts::default()
    };
    assert_eq!(haiku.nanos(&counts(100_000)), Nanos::new(10_000_000));
    assert_eq!(haiku.nanos(&counts(200_000)), Nanos::new(100_000_000));
    let search = list.price("api.search.brave.com", "search").unwrap();
    assert_eq!(search.nanos(&TokenCounts::default()), Nanos::new(5_000_000));
    assert_eq!(list.name, "glave 2026-11-20");
}

proptest::proptest! {
    /// Any price a billionth of a dollar apart is read exactly, never through a float.
    #[test]
    fn decimal_prices_are_read_exactly(
        input in 0_i64..10_i64.pow(13),
        output in 0_i64..10_i64.pow(13),
        read in 0_i64..10_i64.pow(13),
    ) {
        let fields = format!(
            "input = \"{}\"\noutput = \"{}\"\ncache_read = \"{}\"",
            dollars(input),
            dollars(output),
            dollars(read),
        );
        let list = PriceList::read(&entry("s", "m", &fields)).unwrap();
        let price = list.price("s", "m").unwrap();
        proptest::prop_assert_eq!(price.input_per_mtok, Nanos::new(input));
        proptest::prop_assert_eq!(price.output_per_mtok, Nanos::new(output));
        proptest::prop_assert_eq!(price.cache_read_per_mtok, Nanos::new(read));
        proptest::prop_assert_eq!(price.cache_write_per_mtok, Nanos::new(input));
    }

    /// Text that is no list is refused, never read as a price of nothing.
    #[test]
    fn any_text_reads_or_is_refused(text in "\\PC{0,200}") {
        if let Ok(list) = PriceList::read(&text) {
            for entry in list.entries.values() {
                proptest::prop_assert!(entry.check().is_ok());
            }
        }
    }
}
