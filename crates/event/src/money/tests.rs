use proptest::prelude::*;

use super::*;

fn usd(nanos: i64) -> Money {
    Money {
        nanos: Nanos(nanos),
        currency: Currency::USD,
    }
}

#[test]
fn currency_codes_are_validated() {
    assert_eq!(Currency::new("EUR").unwrap().to_string(), "EUR");
    for bad in ["usd", "US", "USDT", "U$D", ""] {
        assert!(Currency::new(bad).is_err(), "{bad}");
    }
}

#[test]
fn money_round_trips_exactly() {
    let cost = usd(4_200_000);
    let json = serde_json::to_string(&cost).unwrap();
    assert_eq!(json, r#"{"nanos":4200000,"currency":"USD"}"#);
    assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), cost);
}

#[test]
fn dollars_parse_exactly_and_past_the_ninth_place_round_away_from_zero() {
    for (text, nanos) in [
        ("2", 2_000_000_000),
        ("0.50", 500_000_000),
        ("4.1", 4_100_000_000),
        ("0.0042", 4_200_000),
        ("-0.000000001", -1),
        ("0.0000000001", 1),
        ("0.0000000000000000000000000000000000000000000001", 1),
        ("-0.0000000001", -1),
        ("12.3456789012", 12_345_678_902),
        ("0.0000000000", 0),
        ("9223372036.854775807", i64::MAX),
        ("-9223372036.854775808", i64::MIN),
    ] {
        assert_eq!(Money::parse_dollars(text).unwrap(), usd(nanos), "{text}");
    }
    for bad in [
        "",
        "-",
        ".5",
        "2.",
        "+2",
        "1e3",
        "4e-7",
        "1.2.3",
        "$2",
        " 2",
        "two",
        "9223372037",
        "9223372036.854775808",
    ] {
        assert!(Money::parse_dollars(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_provider_may_write_dollars_with_an_exponent() {
    for (text, nanos) in [
        ("0.0042", 4_200_000),
        ("1e3", 1_000_000_000_000),
        ("4e-7", 400),
        ("4E-07", 400),
        ("1.5e+2", 150_000_000_000),
        ("1e-300", 1),
        ("0e2000000000", 0),
    ] {
        assert_eq!(Money::reported_dollars(text), Some(usd(nanos)), "{text}");
    }
    for bad in ["1e", "e3", "1e3.5", "1e2000000000", "+2", "two"] {
        assert_eq!(Money::reported_dollars(bad), None, "{bad:?}");
    }
}

#[test]
fn a_price_per_token_is_held_per_million_tokens() {
    let price = |text| Nanos::per_million_tokens(text).map(Nanos::get);
    assert_eq!(price("0.000003"), Some(3_000_000_000));
    assert_eq!(price("3e-6"), Some(3_000_000_000));
    assert_eq!(price("0.000000037476"), Some(37_476_000));
    assert_eq!(price("0"), Some(0));
    assert_eq!(price("2"), Some(2_000_000_000_000_000));
    assert_eq!(price("0.000000000000001"), Some(1), "the fifteenth place");
    assert_eq!(
        price("0.0000000000000001"),
        Some(1),
        "never free by rounding"
    );
    assert_eq!(price("9223.372036854775807"), Some(i64::MAX));
    for bad in ["-1", "9223.372036854775808", "free", ""] {
        assert_eq!(price(bad), None, "{bad:?}");
    }
}

#[test]
fn money_reads_short_or_to_a_precision() {
    assert_eq!(usd(4_200_000).to_string(), "$0.0042");
    assert_eq!(usd(20_000_000_000).to_string(), "$20");
    assert_eq!(usd(-1_500_000_000).to_string(), "-$1.5");
    assert_eq!(usd(1).to_string(), "$0.000000001");
    assert_eq!(format!("{:.2}", usd(1_234_567_000)), "$1.23");
    assert_eq!(format!("{:.2}", usd(1_235_000_000)), "$1.24");
    assert_eq!(format!("{:.4}", usd(7_731_000)), "$0.0077");
    assert_eq!(format!("{:.2}", usd(-4_000_000)), "$0.00");
    assert_eq!(format!("{:.0}", usd(2_500_000_000)), "$3");
    assert_eq!(format!("{:.11}", usd(1)), "$0.00000000100");
    let euros = Money {
        nanos: Nanos(1_500_000_000),
        currency: Currency::new("EUR").unwrap(),
    };
    assert_eq!(euros.to_string(), "1.5 EUR");
    assert_eq!(format!("{:.9}", usd(-1).decimal()), "-0.000000001");
    assert_eq!(
        format!("{:.9}", usd(i64::MIN).decimal()),
        "-9223372036.854775808"
    );
}

#[test]
fn sums_keep_to_one_currency() {
    assert_eq!(usd(1).checked_add(usd(2)), Some(usd(3)));
    let tenth = Money::parse_dollars("0.1").unwrap();
    let ten = (0..10).try_fold(usd(0), |sum, _| sum.checked_add(tenth));
    assert_eq!(ten, Some(Money::parse_dollars("1").unwrap()));
    assert_eq!(usd(i64::MAX).checked_add(usd(1)), None);
    let euros = Money {
        nanos: Nanos(1),
        currency: Currency::new("EUR").unwrap(),
    };
    assert_eq!(usd(1).checked_add(euros), None);
}

#[test]
fn sums_at_the_limits_stop_there() {
    assert_eq!(Nanos::MAX.saturating_add(Nanos(1)), Nanos::MAX);
    assert_eq!(Nanos(i64::MIN).saturating_sub(Nanos(1)), Nanos(i64::MIN));
    assert_eq!(Nanos::MAX.checked_add(Nanos(1)), None);
}

proptest! {
    #[test]
    fn written_dollars_parse_back(nanos in any::<i64>()) {
        let money = usd(nanos);
        prop_assert_eq!(Money::parse_dollars(&money.decimal().to_string()).unwrap(), money);
        prop_assert_eq!(Money::parse_dollars(&format!("{:.9}", money.decimal())).unwrap(), money);
    }

    #[test]
    fn rounding_lands_within_half_a_place(nanos in -10_i64.pow(18)..10_i64.pow(18), places in 0_usize..9) {
        let rounded = Money::parse_dollars(&format!("{:.places$}", usd(nanos).decimal())).unwrap();
        let step = 10_i64.pow(u32::try_from(9 - places).unwrap());
        prop_assert!((rounded.nanos.get() - nanos).abs() * 2 <= step);
        prop_assert_eq!(rounded.nanos.get() % step, 0);
    }

    #[test]
    fn sums_are_exact_or_refused(a in any::<i64>(), b in any::<i64>()) {
        let exact = i64::try_from(i128::from(a) + i128::from(b)).ok();
        prop_assert_eq!(usd(a).checked_add(usd(b)).map(|sum| sum.nanos.get()), exact);
    }

    /// Any decimal a service could list is exactly the integer its digits say, at any length a
    /// float could not hold, and written with an exponent or without.
    #[test]
    fn prices_convert_exactly(digits in 0_u64..10_u64.pow(15), zeros in 0_usize..15) {
        let plain = format!("0.{}{digits:015}", "0".repeat(zeros));
        let exact = digits.div_ceil(10_u64.pow(u32::try_from(zeros).unwrap()));
        let exact = Nanos(i64::try_from(exact).unwrap());
        prop_assert_eq!(Nanos::per_million_tokens(&plain), Some(exact));
        let shifted = format!("{digits}e-{}", 15 + zeros);
        prop_assert_eq!(Nanos::per_million_tokens(&shifted), Some(exact));
    }

    /// Sums of a parsed amount are what its digits say, however many, as binary fractions'
    /// are not: ten times `0.1` is `1`.
    #[test]
    fn sums_do_not_drift(digits in 1_i64..10_i64.pow(9), times in 1_i64..1_000) {
        let amount = Money::parse_dollars(&format!("0.{digits:09}")).unwrap();
        let sum = (0..times).try_fold(usd(0), |sum, _| sum.checked_add(amount)).unwrap();
        prop_assert_eq!(sum, usd(digits * times));
    }

    #[test]
    fn nothing_but_zero_rounds_to_zero(digits in 1_u64.., zeros in 0_usize..200) {
        let text = format!("0.{}{digits}", "0".repeat(zeros));
        prop_assert!(Money::parse_dollars(&text).unwrap().nanos > Nanos::ZERO);
        prop_assert!(Nanos::per_million_tokens(&text).unwrap() > Nanos::ZERO);
    }
}

#[test]
fn cents_round_up_for_what_could_be_spent_and_down_for_what_is_left() {
    assert_eq!(usd(2_400_000_001).cents_up(), usd(2_410_000_000));
    assert_eq!(usd(2_400_000_000).cents_up(), usd(2_400_000_000));
    assert_eq!(usd(1).cents_up(), usd(10_000_000), "under a cent is a cent");
    assert_eq!(usd(1_509_999_999).cents_down(), usd(1_500_000_000));
    assert_eq!(usd(4_200_000).cents_down(), usd(0));
    assert_eq!(usd(-4_200_000).cents_down(), usd(-10_000_000));
}
