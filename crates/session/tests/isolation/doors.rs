//! The ways in over HTTP beside the session API's methods. The table lists every method; this
//! lists every place the HTTP transport reads a request's path, the way in it is, and what
//! keeps it to its caller. A new route, or a new place a path is read, fails here until it is
//! listed with what attacks it, so no way in ships unlisted.

/// The HTTP transport, as it is written.
const HTTP: &str = include_str!("../../src/http.rs");

/// Its other files, which read no request's path: a route there is a new way in too.
const BESIDE: [(&str, &str); 4] = [
    ("http/body.rs", include_str!("../../src/http/body.rs")),
    ("http/hosts.rs", include_str!("../../src/http/hosts.rs")),
    ("http/listen.rs", include_str!("../../src/http/listen.rs")),
    ("http/site.rs", include_str!("../../src/http/site.rs")),
];

/// Each place a request's path is read, as written, and the way in it is.
const DOORS: [(&str, &str); 3] = [
    // The page a browser comes back to from a connection's sign-in. It carries no sign-in of
    // glave's: its `state`, a random value only the browser that began the sign-in was given,
    // names what it finishes, once, and only the person on the server's computer begins one.
    // Attacked by `connections.rs` (`the_browser_comes_back_to_this_server_and_finishes_the_sign_in`,
    // `only_the_person_on_this_computer_signs_in_to_connections`) and `glave`'s `connected.rs`.
    (
        "if request.method() == Method::GET && request.uri().path() == CALLBACK {",
        "GET /connect/callback",
    ),
    // The web app's own files, the same for everyone and for no one signed in: nothing of
    // anyone's. Attacked by `site.rs` (`nothing_outside_the_app_or_but_the_index_page_is_served`,
    // `no_answer_sets_a_cookie`).
    (
        "site.answer(request.uri().path(), accepted).await",
        "GET or HEAD of any other path",
    ),
    // The session API: one JSON-RPC request, its caller the token's. Every method is the
    // table's to list and the sweep's to attack; anything but a POST to `/` is refused
    // (`http.rs`, `the_method_header_must_agree_and_only_post_to_the_root_is_served`).
    ("if request.uri().path() != \"/\" {", "POST /"),
];

#[test]
fn every_way_in_over_http_is_listed() {
    let reads: Vec<&str> = HTTP
        .lines()
        .map(str::trim)
        .filter(|line| line.contains(".path()"))
        .collect();
    for line in &reads {
        assert!(
            DOORS.iter().any(|(read, _)| line == read),
            "the HTTP transport reads a request's path somewhere new: `{line}`. If it is a new \
             way in, list it in DOORS with what attacks it"
        );
    }
    for (read, door) in DOORS {
        let times = reads.iter().filter(|line| **line == read).count();
        assert_eq!(
            times, 1,
            "{door} is no longer read as `{read}`: list it as it is now"
        );
    }
    for (file, text) in BESIDE {
        assert!(
            !text.contains(".uri()"),
            "{file} reads a request's address: if it is a new way in, list it in DOORS"
        );
    }
}
