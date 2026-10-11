//! The web app beside the API: `GET` serves the built app's files, a route of the app is its
//! `index.html` with this load's nonce in place, every answer carries the Content Security
//! Policy with that same nonce, nothing outside the folder is reachable, and `POST /` is still
//! the API, signed in as ever.

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
    use std::path::Path;

    use glave_session::Answering;
    use glave_session::http::{Site, serve_site};
    use glave_session::signin::{LocalToken, SignIn};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    use super::issuer::Fixed;
    use super::support::*;

    const INDEX: &str = r#"<!doctype html><app-root ngCspNonce="GLAVE_CSP_NONCE"></app-root><script src="main.js" nonce="GLAVE_CSP_NONCE"></script>"#;

    /// A built app: index.html, a script, and a file beside the folder it must not reach.
    fn app(dir: &Path) -> std::path::PathBuf {
        let root = dir.join("browser");
        std::fs::create_dir_all(root.join("media")).unwrap();
        std::fs::write(root.join("index.html"), INDEX).unwrap();
        std::fs::write(root.join("main.js"), "console.log(1)").unwrap();
        std::fs::write(root.join("media/font.woff2"), [0u8, 1, 2]).unwrap();
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        root
    }

    /// The nonce a Content-Security-Policy admits styles with.
    fn nonce_of(policy: &str) -> String {
        let (_, after) = policy.split_once("'nonce-").unwrap();
        after.split('\'').next().unwrap().to_owned()
    }

    #[test]
    fn the_app_is_served_beside_the_api_under_a_strict_policy() {
        block_on(async {
            let dir = folder("http-site");
            let site = Site::folder(app(&dir)).unwrap();
            let server = server(&dir, vec![], Answering::Clients).await;
            let signin = SignIn::local(LocalToken::new([7; 32]), Box::new(Fixed));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let (stop, stopped) = oneshot::channel::<()>();
            let serving = tokio::spawn(serve_site(server, signin, Some(site), listener, async {
                let _ = stopped.await;
            }));
            let http = reqwest::Client::new();

            // A route of the app is its index.html, with a fresh nonce in it and in the policy.
            let page = http.get(format!("{url}/t/123")).send().await.unwrap();
            assert_eq!(page.status(), 200);
            let policy = page.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .to_owned();
            assert!(policy.contains("script-src 'self'"), "{policy}");
            assert!(!policy.contains("unsafe-inline"), "{policy}");
            assert_eq!(page.headers()["x-content-type-options"], "nosniff");
            assert_eq!(page.headers()["cache-control"], "no-store");
            let nonce = nonce_of(&policy);
            assert_eq!(nonce.len(), 20);
            let body = page.text().await.unwrap();
            assert!(!body.contains("GLAVE_CSP_NONCE"));
            assert_eq!(body.matches(&nonce).count(), 2);
            let again = http.get(format!("{url}/")).send().await.unwrap();
            let next = nonce_of(again.headers()["content-security-policy"].to_str().unwrap());
            assert_ne!(next, nonce, "every load has its own nonce");

            // Files are served typed; nothing outside the folder or hidden is.
            let script = http.get(format!("{url}/main.js")).send().await.unwrap();
            assert_eq!(script.status(), 200);
            assert_eq!(
                script.headers()["content-type"],
                "text/javascript; charset=utf-8"
            );
            let font = http
                .get(format!("{url}/media/font.woff2"))
                .send()
                .await
                .unwrap();
            assert_eq!(font.headers()["content-type"], "font/woff2");
            assert_eq!(font.bytes().await.unwrap().as_ref(), [0u8, 1, 2]);
            for path in [
                "/missing.js",
                "/../secret.txt",
                "/%2e%2e/secret.txt",
                "/.env",
            ] {
                let refused = http.get(format!("{url}{path}")).send().await.unwrap();
                assert_eq!(refused.status(), 404, "{path}");
            }

            // POST / is the API, which still needs a sign-in.
            let api = http
                .post(format!("{url}/"))
                .body(r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{}}"#)
                .send()
                .await
                .unwrap();
            assert_eq!(api.status(), 401);

            stop.send(()).unwrap();
            serving.await.unwrap().unwrap();
        });
    }

    #[test]
    fn a_folder_without_an_app_is_refused() {
        let dir = folder("http-site-empty");
        let refused = Site::folder(&dir).unwrap_err();
        assert!(refused.to_string().contains("npm run build"));
    }
}
