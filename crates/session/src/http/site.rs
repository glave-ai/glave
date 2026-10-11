//! The web app's files, served beside the API by `glave serve --http`: any `GET` is a file of
//! the built app (`web/`, `npm run build`), and a path that names no file is the app's
//! `index.html`, so its routes load. Every answer carries a strict Content Security Policy: no
//! inline script at all, and inline styles only with the nonce made for that load, which
//! `index.html` hands to Angular (`ngCspNonce`) for the styles it adds.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use hyper::header::{self, HeaderValue};
use hyper::{Response, StatusCode};

use super::body::Answer;

/// The placeholder in `index.html` each load's nonce replaces.
const NONCE_PLACEHOLDER: &str = "GLAVE_CSP_NONCE";

/// The built web app, in a folder.
#[derive(Debug, Clone)]
pub struct Site {
    root: PathBuf,
}

/// Why a folder is not a built web app.
#[derive(Debug, thiserror::Error)]
#[error("{0} has no index.html: build the web app (npm run build in web/) or pass --web")]
pub struct NoSite(String);

impl Site {
    /// The app built into `root`, the folder that holds its `index.html`.
    pub fn folder(root: impl Into<PathBuf>) -> Result<Self, NoSite> {
        let root = root.into();
        if root.join("index.html").is_file() {
            Ok(Self { root })
        } else {
            Err(NoSite(root.display().to_string()))
        }
    }

    /// The answer to `GET path`, with `nonce` for this load's inline styles.
    pub(crate) async fn answer(&self, path: &str, nonce: &str) -> Response<Answer> {
        let Some(relative) = relative(path) else {
            return secured(status(StatusCode::NOT_FOUND), nonce);
        };
        // A path with an extension names a file; anything else is a route of the app.
        if relative.extension().is_some() {
            return match tokio::fs::read(self.root.join(&relative)).await {
                Ok(bytes) => {
                    let mut response = Response::new(Answer::bytes(bytes));
                    set(&mut response, header::CONTENT_TYPE, content_type(&relative));
                    set(&mut response, header::CACHE_CONTROL, "no-cache");
                    secured(response, nonce)
                }
                Err(_) => secured(status(StatusCode::NOT_FOUND), nonce),
            };
        }
        match tokio::fs::read_to_string(self.root.join("index.html")).await {
            Ok(page) => {
                let page = page.replace(NONCE_PLACEHOLDER, nonce);
                let mut response = Response::new(Answer::once(page));
                set(
                    &mut response,
                    header::CONTENT_TYPE,
                    "text/html; charset=utf-8",
                );
                set(&mut response, header::CACHE_CONTROL, "no-store");
                secured(response, nonce)
            }
            Err(_) => secured(status(StatusCode::NOT_FOUND), nonce),
        }
    }
}

/// The policy every answer carries; `nonce` admits the styles Angular adds at run time.
/// web/e2e/serve.mjs serves the end-to-end tests with the same one.
pub(crate) fn policy(nonce: &str) -> String {
    [
        "default-src 'none'",
        "script-src 'self'",
        &format!("style-src 'self' 'nonce-{nonce}'"),
        "font-src 'self'",
        "img-src 'self'",
        "connect-src 'self'",
        "manifest-src 'self'",
        "base-uri 'self'",
        "form-action 'none'",
        "frame-ancestors 'none'",
    ]
    .join("; ")
}

/// A nonce for one load: random bytes as hex.
pub(crate) fn nonce(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// `path` as a path inside the site, or `None` when it would leave it or names a hidden file.
fn relative(path: &str) -> Option<PathBuf> {
    let mut inside = PathBuf::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if part.starts_with('.') || part.contains(['\\', ':', '%', '\0']) {
            return None;
        }
        inside.push(part);
    }
    inside
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(inside)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn status(code: StatusCode) -> Response<Answer> {
    let mut response = Response::new(Answer::empty());
    *response.status_mut() = code;
    response
}

fn set(response: &mut Response<Answer>, name: header::HeaderName, value: &'static str) {
    response
        .headers_mut()
        .insert(name, HeaderValue::from_static(value));
}

/// `response` with the security headers every answer from the site carries.
fn secured(mut response: Response<Answer>, nonce: &str) -> Response<Answer> {
    let headers = response.headers_mut();
    if let Ok(policy) = HeaderValue::from_str(&policy(nonce)) {
        headers.insert(header::CONTENT_SECURITY_POLICY, policy);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_site() {
        assert_eq!(relative("/"), Some(PathBuf::new()));
        assert_eq!(
            relative("/media/font.woff2"),
            Some(PathBuf::from("media/font.woff2"))
        );
        assert_eq!(relative("//main.js"), Some(PathBuf::from("main.js")));
        assert_eq!(relative("/../secret"), None);
        assert_eq!(relative("/a/../../b"), None);
        assert_eq!(relative("/.env"), None);
        assert_eq!(relative("/a%2e%2e/b"), None);
        assert_eq!(relative("/c:/windows"), None);
        assert_eq!(relative("/a\\b"), None);
    }

    #[test]
    fn the_policy_admits_no_inline_script_and_only_nonced_styles() {
        let policy = policy("abc");
        assert!(policy.contains("script-src 'self'"));
        assert!(!policy.contains("unsafe-inline"));
        assert!(policy.contains("style-src 'self' 'nonce-abc'"));
        assert!(policy.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn a_nonce_is_hex() {
        assert_eq!(nonce(&[0, 15, 255]), "000fff");
    }

    #[test]
    fn files_are_typed_by_extension() {
        assert_eq!(
            content_type(Path::new("a.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("a.woff2")), "font/woff2");
        assert_eq!(content_type(Path::new("a.bin")), "application/octet-stream");
    }
}
