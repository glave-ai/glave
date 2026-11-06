use super::*;

const ID: &str = "3e68ede088b4e1866ed530d7461b769395e7f285";

/// One commit as [`FORMAT`] prints it.
fn record(author: &str, committer: &str, message: &str) -> String {
    format!("{ID}\x1f{author}\x1f{committer}\x1f{message}\x1e")
}

fn refused(author: &str, committer: &str, message: &str) -> Vec<String> {
    let log = record(author, committer, message);
    parsed(&log).unwrap().iter().flat_map(refusals).collect()
}

#[test]
fn the_owner_bots_and_addresses_elsewhere_are_allowed() {
    let message = "fix: a thing (#1)\n\n\
                   Co-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\n\
                   Co-authored-by: Glave contributor <contributor@glave.invalid>\n";
    let actions = "41898282+github-actions[bot]@users.noreply.github.com";
    let commits = [
        (
            "seashell86@users.noreply.github.com",
            "noreply@github.com",
            message,
        ),
        (
            "1234+SeaShell86@users.noreply.github.com",
            "a@example.com",
            "x",
        ),
        (actions, actions, "chore: release"),
    ];
    for (author, committer, message) in commits {
        assert_eq!(refused(author, committer, message), Vec::<String>::new());
    }
}

#[test]
fn a_strangers_co_author_trailer_is_refused() {
    let message = "fix(harness): a turn says so (#1197)\n\nBody.\n\n\
                   Co-authored-by: Glave contributor <noreply@users.noreply.github.com>\n\
                   co-authored-by:Someone <42+contributor@Users.Noreply.GitHub.com>\n";
    let found = refused(
        "seashell86@users.noreply.github.com",
        "noreply@github.com",
        message,
    );
    assert_eq!(
        found,
        [
            "3e68ede088 \"fix(harness): a turn says so (#1197)\": its Co-authored-by trailer \
             noreply@users.noreply.github.com is the GitHub account `noreply`",
            "3e68ede088 \"fix(harness): a turn says so (#1197)\": its Co-authored-by trailer \
             42+contributor@Users.Noreply.GitHub.com is the GitHub account `contributor`",
        ]
    );
}

#[test]
fn a_strangers_author_or_committer_is_refused() {
    let stranger = "contributor@users.noreply.github.com";
    let found = refused(stranger, "contributor@glave.invalid", "chore: x");
    assert_eq!(
        found,
        [
            "3e68ede088 \"chore: x\": its author contributor@users.noreply.github.com is the \
             GitHub account `contributor`"
        ]
    );
    let found = refused("contributor@glave.invalid", stranger, "chore: x");
    assert!(
        found.iter().all(|line| line.contains("its committer")),
        "{found:?}"
    );
    assert_eq!(found.len(), 1);
}

#[test]
fn a_log_reads_as_its_commits() {
    let log = format!(
        "{}\n{}\n",
        record(
            "a@example.com",
            "b@example.com",
            "one\n\nCo-Authored-By: C <c@example.com>\n"
        ),
        record("d@example.com", "e@example.com", "two")
    );
    let commits = parsed(&log).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(
        commits.first().unwrap().addresses,
        [
            (Role::Author, "a@example.com"),
            (Role::Committer, "b@example.com"),
            (Role::CoAuthor, "c@example.com"),
        ]
    );
    assert_eq!(commits.get(1).unwrap().subject, "two");
    assert!(parsed("not a commit\x1e").is_err());
    assert_eq!(parsed(""), Ok(Vec::new()));
}

#[test]
fn only_the_no_reply_domain_names_a_login() {
    assert_eq!(login("noreply@users.noreply.github.com"), Some("noreply"));
    assert_eq!(
        login("7+bot[bot]@users.noreply.github.com"),
        Some("bot[bot]")
    );
    assert_eq!(login("x+y@users.noreply.github.com"), Some("x+y"));
    assert_eq!(login("noreply@github.com"), None);
    assert_eq!(login("someone@users.noreply.github.com.example"), None);
}
