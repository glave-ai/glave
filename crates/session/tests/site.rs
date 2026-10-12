//! The web app beside the API: `GET` serves the built app's files, a route of the app is its
//! `index.html` with this load's nonce in place, every answer carries the Content Security
//! Policy with that same nonce, nothing outside the folder is reachable (by `..`, by encoded
//! dots, or by a symbolic link), no page but `index.html` is served as HTML, hashed assets are
//! cached for good, and `POST /` is still the API, signed in as ever.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/issuer.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod issuer;

#[cfg(test)]
mod site {
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};

    use glave_session::Answering;
    use glave_session::http::{self, Site};
    use glave_session::signin::{LocalToken, SignIn};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use super::issuer::fixed_clock;
    use super::support::*;

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

    /// A served site, until `stop` is sent.
    struct Served {
        address: SocketAddr,
        stop: oneshot::Sender<()>,
        serving: JoinHandle<std::io::Result<()>>,
    }

    async fn serve(dir: &PathBuf) -> Served {
        let site = Site::folder(app(dir)).unwrap();
        let server = server(dir, vec![], Answering::Clients).await;
        let signin = SignIn::local(LocalToken::new([7; 32]), Box::new(fixed_clock()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let serving = tokio::spawn(http::serve(server, signin, Some(site), listener, async {
            let _ = stopped.await;
        }));
        Served {
            address,
            stop,
            serving,
        }
    }

    impl Served {
        async fn close(self) {
            self.stop.send(()).unwrap();
            self.serving.await.unwrap().unwrap();
        }
    }

    /// The status line's code for `GET <target>`, sent byte for byte as written: no client
    /// in between to tidy the path.
    async fn raw_status(address: SocketAddr, target: &str) -> u16 {
        let mut stream = TcpStream::connect(address).await.unwrap();
        let request =
            format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
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
            let served = serve(&dir).await;
            let url = format!("http://{}", served.address);
            let http = reqwest::Client::new();

            // A route of the app is its index.html, with a fresh nonce in it and in the policy.
            let page = http.get(format!("{url}/t/123")).send().await.unwrap();
            assert_eq!(page.status(), 200);
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
            assert!(!body.contains("GLAVE_CSP_NONCE"));
            assert_eq!(body.matches(&nonce).count(), 2);
            let again = http.get(format!("{url}/")).send().await.unwrap();
            let next = nonce_of(again.headers()["content-security-policy"].to_str().unwrap());
            assert_ne!(next, nonce, "every load has its own nonce");

            // Files are served typed; hashed ones are cached for good, others revalidated.
            let script = http
                .get(format!("{url}/main-ABCD1234.js"))
                .send()
                .await
                .unwrap();
            assert_eq!(script.status(), 200);
            assert_eq!(
                script.headers()["content-type"],
                "text/javascript; charset=utf-8"
            );
            assert_eq!(
                script.headers()["cache-control"],
                "public, max-age=31536000, immutable"
            );
            let icon = http.get(format!("{url}/favicon.svg")).send().await.unwrap();
            assert_eq!(icon.headers()["cache-control"], "no-cache");
            let font = http
                .get(format!("{url}/media/font.woff2"))
                .send()
                .await
                .unwrap();
            assert_eq!(font.headers()["content-type"], "font/woff2");
            assert_eq!(font.bytes().await.unwrap().as_ref(), [0u8, 1, 2]);

            // POST / is the API, which still needs a sign-in.
            let api = http
                .post(format!("{url}/"))
                .body(r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{}}"#)
                .send()
                .await
                .unwrap();
            assert_eq!(api.status(), 401);
            served.close().await;
        });
    }

    #[test]
    fn nothing_outside_the_folder_or_but_the_index_page_is_served() {
        block_on!(async {
            let dir = folder("http-site-escape");
            let served = serve(&dir).await;
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
                "/link.txt",
            ] {
                assert_eq!(raw_status(served.address, target).await, 404, "{target}");
            }
            assert_eq!(raw_status(served.address, "/index.html").await, 200);
            assert_eq!(raw_status(served.address, "/main-ABCD1234.js").await, 200);
            served.close().await;
        });
    }

    #[test]
    fn a_folder_without_an_app_is_refused() {
        let dir = folder("http-site-empty");
        let refused = Site::folder(&dir).unwrap_err();
        assert!(refused.to_string().contains("npm run build"));
        assert!(Site::folder(dir.join("missing")).is_err());
    }
}
