//! The web app beside the API, from a folder and built into the program alike: `GET` serves
//! the built app's files, a route of the app is its `index.html` with this load's nonce in
//! place, every answer carries the Content Security Policy with that same nonce, nothing outside
//! the app is reachable (by `..`, by encoded dots, or by a symbolic link), no page but
//! `index.html` is served as HTML, hashed assets are cached for good, and `POST /` is still the
//! API, signed in as ever.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use glave_session::Answering;
use glave_session::http::{Files, Site};
use glave_session::signin::{LocalToken, SignIn};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, serve_with};
use crate::support::*;

const INDEX: &str = r#"<!doctype html><app-root ngCspNonce="GLAVE_CSP_NONCE"></app-root><script src="main-ABCD1234.js" nonce="GLAVE_CSP_NONCE"></script>"#;

/// A built app: index.html, a hashed script, a font, a stray page, and a secret beside
/// the folder that it must not reach, by name or by a link.
fn app(dir: &Path) -> PathBuf {
    let root = dir.join("browser");
    std::fs::create_dir_all(root.join("media")).unwrap();
    std::fs::write(root.join("index.html"), INDEX).unwrap();
    std::fs::write(root.join("main-ABCD1234.js"), "console.log(1)").unwrap();
    std::fs::write(root.join("favicon.svg"), "<svg/>").unwrap();
    std::fs::write(root.join("media/font.woff2"), [0u8, 1, 2]).unwrap();
    std::fs::write(root.join("other.html"), "<p>not served</p>").unwrap();
    std::fs::write(dir.join("secret.txt"), "no").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("secret.txt"), root.join("link.txt")).unwrap();
    root
}

/// The same app built into the program, stray page and all.
static BUILT_IN: Files = &[
    ("index.html", INDEX.as_bytes()),
    ("main-ABCD1234.js", b"console.log(1)"),
    ("favicon.svg", b"<svg/>"),
    ("media/font.woff2", &[0, 1, 2]),
    ("other.html", b"<p>not served</p>"),
];

/// The app in a folder made in `dir`, and the app built in.
fn sites(dir: &Path) -> [Site; 2] {
    [
        Site::folder(app(dir)).unwrap(),
        Site::built_in(BUILT_IN).unwrap(),
    ]
}

async fn serve(dir: &Path, site: Site) -> Served {
    let signin = SignIn::local(LocalToken::new([7; 32]), Box::new(fixed_clock()));
    serve_with(
        server(dir, vec![], Answering::Clients).await,
        signin,
        Some(site),
    )
    .await
}

/// The status line's code for `GET <target>`, sent byte for byte as written: no client
/// in between to tidy the path.
async fn raw_status(address: SocketAddr, target: &str) -> u16 {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    let answer = String::from_utf8_lossy(&answer);
    answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap()
}

/// The nonce a Content-Security-Policy admits styles with.
fn nonce_of(policy: &str) -> String {
    let (_, after) = policy.split_once("'nonce-").unwrap();
    after.split('\'').next().unwrap().to_owned()
}

#[test]
fn the_app_is_served_beside_the_api_under_a_strict_policy() {
    block_on!(async {
        let dir = folder("http-site");
        for site in sites(&dir) {
            let shown = format!("{site:?}");
            let served = serve(&dir, site).await;
            pages_carry_the_policy_and_their_nonce(&served, &shown).await;
            files_are_typed_and_cached(&served, &shown).await;
            served.close().await;
        }
    });
}

/// A route of the app, and the page by name, are its index.html with a fresh nonce in it and in
/// the policy; POST / is the API, which still needs a sign-in.
async fn pages_carry_the_policy_and_their_nonce(served: &Served, site: &str) {
    let url = format!("http://{}", served.address);
    let http = reqwest::Client::new();
    let page = http.get(format!("{url}/t/123")).send().await.unwrap();
    assert_eq!(page.status(), 200, "{site}");
    let headers = page.headers().clone();
    let policy = headers["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(policy.contains("script-src 'self'"), "{policy}");
    assert!(!policy.contains("unsafe-inline"), "{policy}");
    assert!(
        policy.contains("require-trusted-types-for 'script'"),
        "{policy}"
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["cross-origin-opener-policy"], "same-origin");
    assert_eq!(headers["cache-control"], "no-store");
    let nonce = nonce_of(&policy);
    assert_eq!(nonce.len(), 32);
    let body = page.text().await.unwrap();
    assert!(!body.contains("GLAVE_CSP_NONCE"), "{site}");
    assert_eq!(body.matches(&nonce).count(), 2, "{site}");
    let again = http.get(format!("{url}/")).send().await.unwrap();
    let next = nonce_of(again.headers()["content-security-policy"].to_str().unwrap());
    assert_ne!(next, nonce, "every load has its own nonce");
    let named = http.get(format!("{url}/index.html")).send().await.unwrap();
    assert_eq!(named.headers()["cache-control"], "no-store", "{site}");
    let named_nonce = nonce_of(named.headers()["content-security-policy"].to_str().unwrap());
    let body = named.text().await.unwrap();
    assert_eq!(body.matches(&named_nonce).count(), 2, "{site}");

    let api = http
        .post(format!("{url}/"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"space/list","params":{}}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(api.status(), 401, "{site}");
}

/// Files are served typed; hashed ones are cached for good, others revalidated.
async fn files_are_typed_and_cached(served: &Served, site: &str) {
    let url = format!("http://{}", served.address);
    let http = reqwest::Client::new();
    let script = http
        .get(format!("{url}/main-ABCD1234.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(script.status(), 200, "{site}");
    assert_eq!(
        script.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        script.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(script.headers().contains_key("content-security-policy"));
    let icon = http.get(format!("{url}/favicon.svg")).send().await.unwrap();
    assert_eq!(icon.headers()["cache-control"], "no-cache", "{site}");
    let font = http
        .get(format!("{url}/media/font.woff2"))
        .send()
        .await
        .unwrap();
    assert_eq!(font.headers()["content-type"], "font/woff2", "{site}");
    assert_eq!(font.bytes().await.unwrap().as_ref(), [0u8, 1, 2]);
}

#[test]
fn nothing_outside_the_app_or_but_the_index_page_is_served() {
    block_on!(async {
        let dir = folder("http-site-escape");
        for site in sites(&dir) {
            let shown = format!("{site:?}");
            let served = serve(&dir, site).await;
            for target in [
                "/../secret.txt",
                "/media/../../secret.txt",
                "/%2e%2e/secret.txt",
                "/%2E%2E%2Fsecret.txt",
                "/.%2e/secret.txt",
                "/..%5csecret.txt",
                "/.env",
                "/other.html",
                "/missing.js",
                "/media/missing-ABCD1234.woff2",
                "/link.txt",
            ] {
                let status = raw_status(served.address, target).await;
                assert_eq!(status, 404, "{target} from {shown}");
            }
            assert_eq!(raw_status(served.address, "/index.html").await, 200);
            assert_eq!(raw_status(served.address, "/main-ABCD1234.js").await, 200);
            served.close().await;
        }
    });
}

#[test]
fn an_app_without_its_page_is_refused() {
    let dir = folder("http-site-empty");
    let refused = Site::folder(&dir).unwrap_err();
    assert!(refused.to_string().contains("npm run build"));
    assert!(Site::folder(dir.join("missing")).is_err());
    let refused = Site::built_in(&[("main-ABCD1234.js", b"console.log(1)")]).unwrap_err();
    assert!(refused.to_string().contains("npm run build"));
}
