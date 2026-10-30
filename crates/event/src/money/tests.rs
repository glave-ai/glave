use proptest::prelude::*;

use super::*;

fn usd(micros: i64) -> Money {
    Money {
        micros,
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
    let cost = usd(4_200);
    let json = serde_json::to_string(&cost).unwrap();
    assert_eq!(json, r#"{"micros":4200,"currency":"USD"}"#);
    assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), cost);
}

#[test]
fn dollars_parse_exactly_and_round_past_the_sixth_place() {
    for (text, micros) in [
        ("2", 2_000_000),
        ("0.50", 500_000),
        ("4.1", 4_100_000),
        ("0.0042", 4_200),
        ("-0.000001", -1),
        ("0.0000005", 1),
        ("0.0000004999", 0),
        ("-0.0000005", -1),
        ("12.3456789", 12_345_679),
    ] {
        assert_eq!(Money::parse_dollars(text).unwrap(), usd(micros), "{text}");
    }
    for bad in [
        "",
        "-",
        ".5",
        "2.",
        "+2",
        "1e3",
        "1.2.3",
        "$2",
        " 2",
        "two",
        "9223372036855",
    ] {
        assert!(Money::parse_dollars(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn money_reads_short_or_to_a_precision() {
    assert_eq!(usd(4_200).to_string(), "$0.0042");
    assert_eq!(usd(20_000_000).to_string(), "$20");
    assert_eq!(usd(-1_500_000).to_string(), "-$1.5");
    assert_eq!(format!("{:.2}", usd(1_234_567)), "$1.23");
    assert_eq!(format!("{:.2}", usd(1_235_000)), "$1.24");
    assert_eq!(format!("{:.4}", usd(7_731)), "$0.0077");
    assert_eq!(format!("{:.2}", usd(-4_000)), "$0.00");
    assert_eq!(format!("{:.0}", usd(2_500_000)), "$3");
    assert_eq!(format!("{:.8}", usd(1)), "$0.00000100");
    let euros = Money {
        micros: 1_500_000,
        currency: Currency::new("EUR").unwrap(),
    };
    assert_eq!(euros.to_string(), "1.5 EUR");
    assert_eq!(format!("{:.6}", usd(-1).decimal()), "-0.000001");
    assert_eq!(
        format!("{:.6}", usd(i64::MIN).decimal()),
        "-9223372036854.775808"
    );
}

#[test]
fn sums_keep_to_one_currency() {
    assert_eq!(usd(1).checked_add(usd(2)), Some(usd(3)));
    assert_eq!(usd(i64::MAX).checked_add(usd(1)), None);
    let euros = Money {
        micros: 1,
        currency: Currency::new("EUR").unwrap(),
    };
    assert_eq!(usd(1).checked_add(euros), None);
}

proptest! {
    #[test]
    fn written_dollars_parse_back(micros in any::<i64>().prop_filter("negatable", |m| *m != i64::MIN)) {
        let money = usd(micros);
        prop_assert_eq!(Money::parse_dollars(&money.decimal().to_string()).unwrap(), money);
        prop_assert_eq!(Money::parse_dollars(&format!("{:.6}", money.decimal())).unwrap(), money);
    }

    #[test]
    fn rounding_lands_within_half_a_place(micros in -10_i64.pow(15)..10_i64.pow(15), places in 0_usize..6) {
        let rounded = Money::parse_dollars(&format!("{:.places$}", usd(micros).decimal())).unwrap();
        let step = 10_i64.pow(u32::try_from(6 - places).unwrap());
        prop_assert!((rounded.micros - micros).abs() * 2 <= step);
        prop_assert_eq!(rounded.micros % step, 0);
    }

    #[test]
    fn sums_are_exact_or_refused(a in any::<i64>(), b in any::<i64>()) {
        let exact = i64::try_from(i128::from(a) + i128::from(b)).ok();
        prop_assert_eq!(usd(a).checked_add(usd(b)).map(|sum| sum.micros), exact);
    }
}

#[test]
fn cents_round_up_for_what_could_be_spent_and_down_for_what_is_left() {
    assert_eq!(usd(2_400_001).cents_up(), usd(2_410_000));
    assert_eq!(usd(2_400_000).cents_up(), usd(2_400_000));
    assert_eq!(usd(4_200).cents_up(), usd(10_000), "under a cent is a cent");
    assert_eq!(usd(1_509_999).cents_down(), usd(1_500_000));
    assert_eq!(usd(4_200).cents_down(), usd(0));
    assert_eq!(usd(-4_200).cents_down(), usd(-10_000));
}
