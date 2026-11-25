//! The ways in over HTTP beside the session API's methods. The table lists every method; this
//! lists every place the HTTP transport routes a request by what it asks for (its address, its
//! query or its method), and every turn the web app's files take on the path they are given,
//! with the way in each is and what keeps it to its caller. Every file of the transport is read
//! as it is now, so a new route, in a new file or an old one, fails here until it is listed
//! with what attacks it, and no way in ships unlisted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a line that routes a request reads of it: its address and the parts of it, or how it
/// was asked.
const ROUTED_BY: [&str; 6] = [
    ".uri()",
    ".path()",
    ".query()",
    ".path_and_query()",
    ".method()",
    "Method::",
];

/// The web app's files (`http/site.rs`): the functions that turn a path into an answer, each of
/// whose turns is a route.
const SITE: &str = "http/site.rs";
const SITE_ROUTES: [&str; 3] = [
    "pub(crate) async fn answer(",
    "async fn file(",
    "async fn read(",
];

/// The way in through which the web app's files are served, the same for everyone and for no
/// one signed in: nothing of anyone's. Attacked by `site.rs`
/// (`nothing_outside_the_app_or_but_the_index_page_is_served`, `no_answer_sets_a_cookie`).
const FILES: &str = "GET or HEAD of any other path: the web app's own files";

/// The way in at a share's own name: its share alone, to anyone with its link.
const SHARED: &str =
    "GET or HEAD at a share's own name: its page, file and download; for people, POST of its forms";

/// Each place a request is routed, by file and as written, and the way in it is.
const DOORS: [(&str, &str, &str); 32] = [
    // A shared file at its link's own name under the share domain: its page, its file and its
    // download, to anyone with the link, nothing of glave's there and no sign-in read. Attacked
    // by `shares.rs` (`a_link_serves_its_share_alone_at_a_name_of_its_own_and_stops_at_once`).
    (
        "http/shared.rs",
        "let (method, path) = (request.method().clone(), request.uri().path().to_owned());",
        SHARED,
    ),
    (
        "http/shared.rs",
        "let posted = method == Method::POST;",
        SHARED,
    ),
    ("http/shared.rs", "let head = method == Method::HEAD;", SHARED),
    (
        "http/shared.rs",
        SHARED,
    ),
    (
        "http/shared.rs",
        "if !posted && method != Method::GET && !head {",
        SHARED,
    ),
    // A share for specific people: a form posted to its own name, an address for a link or a
    // link's token for a proof. Attacked by `shares_people.rs`.
    (
        "http/shared/asking.rs",
        "let path = request.uri().path().to_owned();",
        SHARED,
    ),
    // The page a browser comes back to from a connection's sign-in. It carries no sign-in of
    // glave's: its `state`, a random value only the browser that began the sign-in was given,
    // names what it finishes, once, and only the person on the server's computer begins one.
    // Attacked by `connections.rs` (`the_browser_comes_back_to_this_server_and_finishes_the_sign_in`,
    // `only_the_person_on_this_computer_signs_in_to_connections`) and `glave`'s `connected.rs`.
    (
        "http.rs",
        "if request.method() == Method::GET && request.uri().path() == CALLBACK {",
        "GET /connect/callback",
    ),
    (
        "http.rs",
        "let query = request.uri().query().unwrap_or_default();",
        "GET /connect/callback: its state",
    ),
    // The web app's own files.
    (
        "http.rs",
        "Some(site) if is_read(request.method()) => {",
        FILES,
    ),
    (
        "http.rs",
        "site.answer(request.uri().path(), accepted).await",
        FILES,
    ),
    (
        "http.rs",
        "*method == Method::GET || *method == Method::HEAD",
        FILES,
    ),
    // The session API: one JSON-RPC request, its caller the token's. Every method is the
    // table's to list and the sweep's to attack; anything but a POST to `/` is refused
    // (`http.rs`, `the_method_header_must_agree_and_only_post_to_the_root_is_served`), and the
    // `Glave-Method` header, when sent, must name the request's own method.
    ("http.rs", "if request.uri().path() != \"/\" {", "POST /"),
    ("http.rs", "if request.method() != Method::POST {", "POST /"),
    (
        "http.rs",
        "&& named != request.method()",
        "POST /: the method its header names",
    ),
    (
        "http.rs",
        "request.method()",
        "POST /: the method its header names",
    ),
    // How the web app's files turn a path into an answer: no page but the app's own, never one
    // without this load's nonce, and only a file inside the app's folder or among its files.
    (
        SITE,
        "let Some(nonce) = fresh_nonce() else {",
        "the app's page, never without its nonce",
    ),
    (
        SITE,
        "let Some(relative) = relative(path) else {",
        "a path that leaves the app's files: not found",
    ),
    (
        SITE,
        "if relative.extension().is_some() && relative != Path::new(INDEX) {",
        "a path with an extension: one of the app's files",
    ),
    (
        SITE,
        "match self.read(Path::new(INDEX)).await {",
        "any other path: the app's page",
    ),
    (
        SITE,
        "Some(Found::Plain(page)) => {",
        "any other path: the app's page",
    ),
    (
        SITE,
        "Some(Found::Gzip(_)) | None => secured(status(StatusCode::NOT_FOUND), &policy),",
        "any other path, with no page to serve: not found",
    ),
    (SITE, "if relative", "no .html file but the app's page"),
    (
        SITE,
        "let mut response = match self.read(relative).await {",
        "one of the app's files",
    ),
    (
        SITE,
        "None => return status(StatusCode::NOT_FOUND),",
        "a file the app has not: not found",
    ),
    (
        SITE,
        "Some(Found::Plain(bytes)) => Response::new(Answer::bytes(bytes)),",
        "one of the app's files",
    ),
    (
        SITE,
        "Some(Found::Gzip(bytes)) => {",
        "one of the app's files, gzipped",
    ),
    (
        SITE,
        "if !takes_gzip(accepted) {",
        "one of the app's files, gzipped, to a client that takes none: refused",
    ),
    (
        SITE,
        "match &self.source {",
        "one of the app's files: from its folder or from memory",
    ),
    (
        SITE,
        "if !resolved.starts_with(root) {",
        "one of the app's files: only inside its folder, links followed",
    ),
    (
        SITE,
        "Source::Folder(root) => {",
        "one of the app's files: from its folder",
    ),
    (
        SITE,
        "Source::Memory(files) => {",
        "one of the app's files: from memory",
    ),
    (
        SITE,
        ".map(|&(_, stored)| match stored {",
        "one of the app's files: from memory",
    ),
    (
        SITE,
        "Stored::Plain(bytes) => Found::Plain(Bytes::from_static(bytes)),",
        "one of the app's files: from memory",
    ),
    (
        SITE,
        "Stored::Gzip(bytes) => Found::Gzip(Bytes::from_static(bytes)),",
        "one of the app's files: from memory, gzipped",
    ),
];

/// The transport's files under `src`, by their path from there: `http.rs` and everything in
/// `http/`, as they are now.
fn transport() -> BTreeMap<String, String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = BTreeMap::new();
    let found = rust_files(vec![src.join("http")], vec![src.join("http.rs")]);
    for path in found {
        let name = named(&src, &path);
        files.insert(name, std::fs::read_to_string(&path).unwrap());
    }
    files
}

/// `path` from `src`, with `/` between its parts on every system.
/// The Rust files in `folders` and theirs, after `found`.
pub(super) fn rust_files(mut folders: Vec<PathBuf>, mut found: Vec<PathBuf>) -> Vec<PathBuf> {
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|it| it == "rs") {
                found.push(path);
            }
        }
    }
    found
}

fn named(src: &Path, path: &Path) -> String {
    let relative: PathBuf = path.strip_prefix(src).unwrap().to_path_buf();
    let parts: Vec<&str> = relative.iter().map(|part| part.to_str().unwrap()).collect();
    parts.join("/")
}

/// The lines of `text` that route a request, trimmed, comments left out.
fn routing(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter(|line| ROUTED_BY.iter().any(|read| line.contains(read)))
        .collect()
}

/// The turns the function that `signature` opens in `text` takes, trimmed: each branch, match
/// and arm in its body. None when no function opens so.
fn turns<'a>(text: &'a str, signature: &str) -> Option<Vec<&'a str>> {
    let mut lines = text
        .lines()
        .skip_while(|line| !line.trim().starts_with(signature));
    let opened = lines.next()?;
    let indent = opened.len() - opened.trim_start().len();
    let closing = format!("{}}}", " ".repeat(indent));
    let body = lines.take_while(|line| *line != closing).map(str::trim);
    Some(
        body.filter(|line| !line.starts_with("//"))
            .filter(|line| {
                line.starts_with("if ")
                    || line.starts_with("} else")
                    || line.contains("match ")
                    || line.ends_with("else {")
                    || line.contains(" => ")
            })
            .collect(),
    )
}

#[test]
fn every_way_in_over_http_is_listed() {
    let files = transport();
    assert!(
        files.contains_key("http.rs") && files.contains_key(SITE),
        "the transport's files are not where this reads them: {:?}",
        files.keys().collect::<Vec<_>>()
    );
    let mut reads: Vec<(&str, &str)> = Vec::new();
    for (file, text) in &files {
        reads.extend(routing(text).into_iter().map(|line| (file.as_str(), line)));
    }
    for signature in SITE_ROUTES {
        let found = turns(&files[SITE], signature)
            .unwrap_or_else(|| panic!("{SITE} has no `{signature}`: list how it routes now"));
        reads.extend(found.into_iter().map(|line| (SITE, line)));
    }
    assert!(!reads.is_empty(), "no route found: this reads nothing");
    assert!(!DOORS.is_empty(), "no way in listed: this proves nothing");
    for (file, line) in &reads {
        assert!(
            DOORS.iter().any(|(at, read, _)| at == file && line == read),
            "{file} routes a request somewhere new: `{line}`. If it is a new way in, list it \
             in DOORS with what attacks it"
        );
    }
    for (file, read, door) in DOORS {
        let times = reads.iter().filter(|it| **it == (file, read)).count();
        assert_eq!(
            times, 1,
            "{door} is no longer routed in {file} as `{read}`: list it as it is now"
        );
    }
}
