use super::*;

/// The time each test judges at, injected: the check reads the clock only through SystemClock.
fn now() -> Timestamp {
    "2026-10-26T12:00:00Z".parse().unwrap()
}

fn package(name: &str, version: &str) -> Pinned {
    Pinned {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

fn new(registry: Registry, name: &str, version: &str, replaces: &[&str]) -> New {
    New {
        registry,
        added: Added {
            package: package(name, version),
            replaces: replaces.iter().map(|old| (*old).to_owned()).collect(),
        },
    }
}

/// When something published `hours` before [`now`] was published.
fn hours_ago(hours: i64) -> Timestamp {
    now()
        .checked_sub(SignedDuration::from_hours(hours))
        .unwrap()
}

#[test]
fn a_version_seven_days_old_or_older_comes_in() {
    let new = [
        new(Registry::Crates, "serde", "1.0.229", &["1.0.228"]),
        new(Registry::Npm, "marked", "18.0.14", &[]),
    ];
    let published = BTreeMap::from([
        (
            (Registry::Crates, package("serde", "1.0.229")),
            hours_ago(7 * 24),
        ),
        (
            (Registry::Npm, package("marked", "18.0.14")),
            hours_ago(40 * 24),
        ),
    ]);
    assert_eq!(judged(&new, &[], &published, now()), Verdict::default());
}

#[test]
fn a_version_under_seven_days_old_is_refused_with_when_it_may_come_in() {
    let new = [
        new(Registry::Crates, "tokio", "1.48.0", &["1.47.1"]),
        new(Registry::Npm, "@angular/core", "22.2.3", &[]),
    ];
    let published = BTreeMap::from([
        (
            (Registry::Crates, package("tokio", "1.48.0")),
            hours_ago(2 * 24 + 5),
        ),
        (
            (Registry::Npm, package("@angular/core", "22.2.3")),
            hours_ago(7 * 24 - 1),
        ),
    ]);
    let verdict = judged(&new, &[], &published, now());
    assert_eq!(
        verdict.too_new,
        [
            "Cargo.lock: tokio 1.48.0, replacing 1.47.1, was published 2 days ago, at \
             2026-10-24 07:00 UTC; it may come in from 2026-10-31 07:00 UTC",
            "web/package-lock.json: @angular/core 22.2.3 was published 6 days ago, at \
             2026-10-19 13:00 UTC; it may come in from 2026-10-26 13:00 UTC",
        ]
    );
    assert_eq!(verdict.early, 0);
}

#[test]
fn an_allowed_version_comes_in_early_from_its_own_registry_only() {
    let new = [
        new(Registry::Crates, "semver", "1.0.27", &["1.0.26"]),
        new(Registry::Npm, "semver", "1.0.27", &[]),
    ];
    let published = BTreeMap::from([
        (
            (Registry::Crates, package("semver", "1.0.27")),
            hours_ago(3),
        ),
        ((Registry::Npm, package("semver", "1.0.27")), hours_ago(3)),
    ]);
    let allowed = [(
        Registry::Crates,
        "semver",
        "1.0.27",
        "RUSTSEC-2026-0001: fixed here",
    )];
    let verdict = judged(&new, &allowed, &published, now());
    assert_eq!(verdict.early, 1);
    assert_eq!(verdict.too_new.len(), 1, "{:?}", verdict.too_new);
    assert!(
        verdict.too_new.iter().all(|problem| problem
            .starts_with("web/package-lock.json: semver 1.0.27 was published 3 hours ago")),
        "{:?}",
        verdict.too_new
    );
}

#[test]
fn an_allowed_version_old_enough_now_fails_until_its_entry_goes() {
    let published = BTreeMap::from([
        (
            (Registry::Crates, package("tokio", "1.48.1")),
            hours_ago(9 * 24),
        ),
        ((Registry::Npm, package("marked", "18.0.15")), hours_ago(24)),
    ]);
    let allowed = [
        (
            Registry::Crates,
            "tokio",
            "1.48.1",
            "RUSTSEC-2026-0002: a fix that can't wait",
        ),
        (
            Registry::Npm,
            "marked",
            "18.0.15",
            "GHSA-xxxx: a fix that can't wait, still young",
        ),
        // Not dated: why is a problem of its own, said where it was asked.
        (
            Registry::Npm,
            "gone",
            "1.0.0",
            "a version the registry no longer lists",
        ),
    ];
    let verdict = judged(&[], &allowed, &published, now());
    assert_eq!(
        verdict.stale,
        ["tokio 1.48.1 from crates.io, published 9 days ago, at 2026-10-17 12:00 UTC"]
    );
    assert_eq!(
        verdict.refusal(&[]),
        Some(format!(
            "{ALLOW_LIST} lets in early 1 version old enough now: remove its entry\n  tokio \
             1.48.1 from crates.io, published 9 days ago, at 2026-10-17 12:00 UTC"
        ))
    );
}

#[test]
fn a_refusal_says_what_is_wrong_and_what_to_do_and_a_pass_says_nothing() {
    assert_eq!(Verdict::default().refusal(&[]), None);
    let verdict = Verdict {
        too_new: vec!["Cargo.lock: tokio 1.48.0 was published 2 days ago".to_owned()],
        stale: vec![
            "a 1.0.0 from crates.io".to_owned(),
            "b 2.0.0 from crates.io".to_owned(),
        ],
        early: 0,
    };
    let refusal = verdict
        .refusal(&["crates.io has no gone 9.9.9".to_owned()])
        .unwrap();
    assert_eq!(
        refusal.lines().collect::<Vec<_>>(),
        [
            "1 package version is too new to come in:",
            "  Cargo.lock: tokio 1.48.0 was published 2 days ago",
            "A package version must be 7 days old before it comes in, so a poisoned release is \
             found and pulled first. Wait until then, or, for a security fix that can't wait, \
             list it in xtask/src/package_age/allowed.rs with its reason.",
            "1 package version can't be dated:",
            "  crates.io has no gone 9.9.9",
            "xtask/src/package_age/allowed.rs lets in early 2 versions old enough now: remove \
             their entries",
            "  a 1.0.0 from crates.io",
            "  b 2.0.0 from crates.io",
        ]
    );
}

#[test]
fn a_version_not_dated_is_passed_over_as_its_own_problem_says_why() {
    let new = [new(Registry::Crates, "unlisted", "9.9.9", &[])];
    assert_eq!(
        judged(&new, &[], &BTreeMap::new(), now()),
        Verdict::default()
    );
}

#[test]
fn every_version_new_or_allowed_is_asked_about_once_per_package() {
    let new = [
        new(Registry::Npm, "rxjs", "7.8.2", &[]),
        new(Registry::Npm, "rxjs", "8.0.0", &[]),
        new(Registry::Crates, "rxjs", "0.1.0", &[]),
    ];
    let allowed = [(
        Registry::Npm,
        "rxjs",
        "8.0.1",
        "a fix that can't wait for its week",
    )];
    let wanted = wanted(&new, &allowed);
    let versions = |registry, name: &str| {
        wanted
            .get(&(registry, name.to_owned()))
            .map(|versions| versions.iter().map(String::as_str).collect::<Vec<_>>())
    };
    assert_eq!(
        versions(Registry::Npm, "rxjs"),
        Some(vec!["7.8.2", "8.0.0", "8.0.1"])
    );
    assert_eq!(versions(Registry::Crates, "rxjs"), Some(vec!["0.1.0"]));
    assert_eq!(wanted.len(), 2);
}

#[test]
fn ages_are_said_in_their_largest_whole_unit() {
    assert_eq!(ago(SignedDuration::from_hours(6 * 24 + 23)), "6 days ago");
    assert_eq!(ago(SignedDuration::from_hours(24)), "1 day ago");
    assert_eq!(ago(SignedDuration::from_hours(5)), "5 hours ago");
    assert_eq!(ago(SignedDuration::from_mins(1)), "1 minute ago");
    assert_eq!(ago(SignedDuration::from_secs(59)), "under a minute ago");
    // Published after the clock's now, as a skewed clock may say.
    assert_eq!(ago(SignedDuration::from_secs(-30)), "under a minute ago");
}

#[test]
fn what_passed_is_said_in_a_sentence() {
    let base = "the pull request's base (3e68ede088)";
    assert_eq!(
        summary(0, 0, base),
        "no package version is new since the pull request's base (3e68ede088)"
    );
    assert_eq!(
        summary(1, 0, base),
        "1 package version new since the pull request's base (3e68ede088), published at least \
         7 days ago"
    );
    assert_eq!(
        summary(3, 1, base),
        format!(
            "3 package versions new since the pull request's base (3e68ede088): 2 published at \
             least 7 days ago, and 1 let in early by {ALLOW_LIST}"
        )
    );
}
