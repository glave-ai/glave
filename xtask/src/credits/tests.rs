use super::*;

const ID: &str = "3e68ede088b4e1866ed530d7461b769395e7f285";

const OWNER: (&str, &str) = ("Owner", "2745864+seashell86@users.noreply.github.com");
const GITHUB: (&str, &str) = ("GitHub", "noreply@github.com");

/// One commit as [`FORMAT`] prints it, from its author's and committer's names and addresses.
fn record(author: (&str, &str), committer: (&str, &str), message: &str) -> String {
    let ((an, ae), (cn, ce)) = (author, committer);
    format!("{ID}\x1f{an}\x1f{ae}\x1f{cn}\x1f{ce}\x1f{message}\x1e")
}

fn refused(author: (&str, &str), committer: (&str, &str), message: &str) -> Vec<String> {
    let log = record(author, committer, message);
    parsed(&log).unwrap().iter().flat_map(refusals).collect()
}

#[test]
fn the_owner_bots_and_addresses_elsewhere_are_allowed() {
    let message = "fix: a thing (#1)\n\n\
                   Co-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\n\
                   Co-authored-by: Someone <someone@example.com>\n";
    let squashed = ("Owner", "SeaShell86@users.noreply.github.com");
    let actions = (
        "github-actions[bot]",
        "41898282+github-actions[bot]@users.noreply.github.com",
    );
    let commits = [
        (squashed, GITHUB, message),
        (OWNER, OWNER, "x"),
        (actions, actions, "chore: release"),
    ];
    for (author, committer, message) in commits {
        assert_eq!(refused(author, committer, message), Vec::<String>::new());
    }
}

#[test]
fn a_strangers_co_author_trailer_is_refused() {
    let message = "fix(harness): a turn says so (#1197)\n\nBody.\n\n\
                   Co-authored-by: Someone <noreply@users.noreply.github.com>\n\
                   co-authored-by:Someone <42+contributor@Users.Noreply.GitHub.com>\n";
    assert_eq!(
        refused(OWNER, GITHUB, message),
        [
            "3e68ede088 \"fix(harness): a turn says so (#1197)\": its Co-authored-by trailer \
             Someone <noreply@users.noreply.github.com> is the GitHub account `noreply`",
            "3e68ede088 \"fix(harness): a turn says so (#1197)\": its Co-authored-by trailer \
             Someone <42+contributor@Users.Noreply.GitHub.com> is the GitHub account \
             `contributor`",
        ]
    );
}

#[test]
fn another_account_of_the_owners_login_is_refused() {
    let other = ("Owner", "1234+seashell86@users.noreply.github.com");
    assert_eq!(refused(other, OWNER, "x").len(), 1);
}

#[test]
fn a_strangers_author_or_committer_is_refused() {
    let stranger = ("Someone", "contributor@users.noreply.github.com");
    assert_eq!(
        refused(stranger, OWNER, "chore: x"),
        [
            "3e68ede088 \"chore: x\": its author Someone <contributor@users.noreply.github.com> \
          is the GitHub account `contributor`"
        ]
    );
    let found = refused(OWNER, stranger, "chore: x");
    assert_eq!(found.len(), 1);
    assert!(
        found.iter().all(|line| line.contains("its committer")),
        "{found:?}"
    );
}

#[test]
fn the_identity_agents_once_committed_as_is_refused_wherever_it_stands() {
    let message = "chore: x\n\n\
                   Co-authored-by: Glave contributor <seashell86@users.noreply.github.com>\n\
                   Co-authored-by: Anyone <contributor@glave.invalid>\n";
    let found = refused(OLD_AGENT, OWNER, message);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found
            .iter()
            .all(|line| line.ends_with("is the identity agents once committed as")),
        "{found:?}"
    );
}

#[test]
fn a_log_reads_as_its_commits() {
    let log = format!(
        "{}\n{}\n",
        record(
            ("A", "a@example.com"),
            ("B", "b@example.com"),
            "one\n\nCo-Authored-By: C D <c@example.com>\n"
        ),
        record(("E", "e@example.com"), ("F", "f@example.com"), "two")
    );
    let commits = parsed(&log).unwrap();
    assert_eq!(commits.len(), 2);
    let named = |role, name, address| Named {
        role,
        name,
        address,
    };
    assert_eq!(
        commits.first().unwrap().named,
        [
            named(Role::Author, "A", "a@example.com"),
            named(Role::Committer, "B", "b@example.com"),
            named(Role::CoAuthor, "C D", "c@example.com"),
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
