//! The web app's files, served beside the API by `glave serve --http`: any `GET` is a file of
//! the built app (`web/`, `npm run build`), and a path that names no file is the app's
//! `index.html`, so its routes load. Every answer carries a strict Content Security Policy: no
//! inline script at all, Trusted Types for what script there is, and inline styles only with
//! the nonce made for that load, which `index.html` hands to Angular (`ngCspNonce`) for the
//! styles it adds.
//!
//! The files are in a folder, or built into the program and served from memory, under the same
//! rules. A folder is served to anyone who can reach the listener, without sign-in, so it must
//! hold only the built app and must not be writable by an agent: a file put there would be
//! served from glave's own origin. Only files inside it are served: a symbolic link that leads
//! out of it is refused. From either, no `.html` page but `index.html` is served.

use std::fmt::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use hyper::body::Bytes;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::{Response, StatusCode};

use super::body::Answer;

/// The placeholder in `index.html` each load's nonce replaces.
const NONCE_PLACEHOLDER: &str = "GLAVE_CSP_NONCE";

/// Random bytes in a nonce.
const NONCE_BYTES: usize = 16;

/// The one page the site serves as HTML.
const INDEX: &str = "index.html";

/// A built web app's files as a program carries them (`include_bytes!`): each one's path
/// inside the app, its parts joined by `/` (`media/font-ABCD1234.woff2`), and its bytes.
pub type Files = &'static [(&'static str, &'static [u8])];

/// The built web app.
#[derive(Clone)]
pub struct Site {
    source: Source,
}

/// Where a site's files are.
#[derive(Clone)]
enum Source {
    /// A folder, canonical: every file served must resolve inside it.
    Folder(PathBuf),
    /// Built into the program.
    Memory(Files),
}

/// Why a folder, or the files built into a program, are not a built web app.
#[derive(Debug, thiserror::Error)]
#[error("{0} has no index.html: build the web app (npm run build in web/) or pass --web")]
pub struct NoSite(String);

impl Site {
    /// The app built into `root`, the folder that holds its `index.html`.
    pub fn folder(root: impl Into<PathBuf>) -> Result<Self, NoSite> {
        let root = root.into();
        let shown = root.display().to_string();
        let root = std::fs::canonicalize(&root).map_err(|_| NoSite(shown.clone()))?;
        if root.join(INDEX).is_file() {
            Ok(Self {
                source: Source::Folder(root),
            })
        } else {
            Err(NoSite(shown))
        }
    }

    /// The app in `files`, built into the program, which must hold its `index.html`.
    pub fn built_in(files: Files) -> Result<Self, NoSite> {
        if files.iter().any(|(path, _)| *path == INDEX) {
            Ok(Self {
                source: Source::Memory(files),
            })
        } else {
            Err(NoSite("the web app built into glave".into()))
        }
    }

    /// The answer to `GET path`: a file, or `index.html` for a route of the app.
    pub(crate) async fn answer(&self, path: &str) -> Response<Answer> {
        // Without randomness there is no nonce, and without a nonce no page is safe to serve.
        let Some(nonce) = fresh_nonce() else {
            return status(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let Some(relative) = relative(path) else {
            return secured(status(StatusCode::NOT_FOUND), &nonce);
        };
        // A path with an extension names a file; anything else is a route of the app, and so
        // is the page itself, which is never served without this load's nonce.
        if relative.extension().is_some() && relative != Path::new(INDEX) {
            return secured(self.file(&relative).await, &nonce);
        }
        match self.read(Path::new(INDEX)).await {
            Some(page) => {
                let page = String::from_utf8_lossy(&page).replace(NONCE_PLACEHOLDER, &nonce);
                let mut response = Response::new(Answer::once(page));
                set(
                    &mut response,
                    header::CONTENT_TYPE,
                    "text/html; charset=utf-8",
                );
                set(&mut response, header::CACHE_CONTROL, "no-store");
                secured(response, &nonce)
            }
            None => secured(status(StatusCode::NOT_FOUND), &nonce),
        }
    }

    /// A file of the app, or 404. No `.html` page is served this way: `index.html` is the
    /// app's routes.
    async fn file(&self, relative: &Path) -> Response<Answer> {
        if relative
            .extension()
            .is_some_and(|extension| extension == "html")
        {
            return status(StatusCode::NOT_FOUND);
        }
        let Some(bytes) = self.read(relative).await else {
            return status(StatusCode::NOT_FOUND);
        };
        let mut response = Response::new(Answer::bytes(bytes));
        set(&mut response, header::CONTENT_TYPE, content_type(relative));
        set(
            &mut response,
            header::CACHE_CONTROL,
            cache_control(relative),
        );
        response
    }

    /// The bytes of the file at `relative`: from memory, or, links followed, a file that
    /// resolves inside the folder.
    async fn read(&self, relative: &Path) -> Option<Bytes> {
        match &self.source {
            Source::Folder(root) => {
                let resolved = tokio::fs::canonicalize(root.join(relative)).await.ok()?;
                if !resolved.starts_with(root) {
                    return None;
                }
                tokio::fs::read(resolved).await.ok().map(Bytes::from)
            }
            Source::Memory(files) => {
                let parts: Option<Vec<&str>> = relative.iter().map(|part| part.to_str()).collect();
                let wanted = parts?.join("/");
                files
                    .iter()
                    .find(|(path, _)| *path == wanted)
                    .map(|&(_, bytes)| Bytes::from_static(bytes))
            }
        }
    }
}

impl fmt::Debug for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Source::Folder(root) => write!(f, "Site({})", root.display()),
            Source::Memory(files) => write!(f, "Site(built in, {} files)", files.len()),
        }
    }
}

/// The policy every answer carries; `nonce` admits the styles Angular adds at run time.
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
        "require-trusted-types-for 'script'",
        "trusted-types angular angular#bundler",
    ]
    .join("; ")
}

/// A new nonce from the operating system's randomness, as hex; `None` if it has none to give.
fn fresh_nonce() -> Option<String> {
    let mut bytes = [0u8; NONCE_BYTES];
    getrandom::fill(&mut bytes).ok()?;
    Some(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
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

/// Whether a file's name carries its content hash, as the build names every asset
/// (`main-HFNTQ2NX.js`, `media/font-ABCD1234.woff2`): its content never changes.
fn is_hashed(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    stem.rsplit_once('-').is_some_and(|(_, hash)| {
        hash.len() == 8
            && hash
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    })
}

fn cache_control(path: &Path) -> &'static str {
    if is_hashed(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("webmanifest") => "application/manifest+json",
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
    set(&mut response, header::CACHE_CONTROL, "no-store");
    response
}

fn set(response: &mut Response<Answer>, name: HeaderName, value: &'static str) {
    response
        .headers_mut()
        .insert(name, HeaderValue::from_static(value));
}

/// `response` with the security headers every answer from the site carries.
fn secured(mut response: Response<Answer>, nonce: &str) -> Response<Answer> {
    if let Ok(policy) = HeaderValue::from_str(&policy(nonce)) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, policy);
    }
    set(&mut response, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(&mut response, header::REFERRER_POLICY, "no-referrer");
    set(&mut response, header::X_FRAME_OPTIONS, "DENY");
    set(
        &mut response,
        HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    );
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
        assert!(policy.contains("manifest-src 'self'"));
        assert!(policy.contains("require-trusted-types-for 'script'"));
    }

    #[test]
    fn a_nonce_is_sixteen_random_bytes_in_hex() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        let nonce = fresh_nonce().unwrap();
        assert_eq!(nonce.len(), NONCE_BYTES * 2);
        assert_ne!(fresh_nonce().unwrap(), nonce);
    }

    #[test]
    fn hashed_assets_are_cached_for_good_and_others_revalidated() {
        assert_eq!(
            cache_control(Path::new("main-HFNTQ2NX.js")),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(
            cache_control(Path::new("media/atkinson-latin-400-ABCD1234.woff2")),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control(Path::new("favicon.svg")), "no-cache");
        assert_eq!(cache_control(Path::new("my-file.js")), "no-cache");
    }

    #[test]
    fn files_are_typed_by_extension() {
        assert_eq!(
            content_type(Path::new("a.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("a.woff2")), "font/woff2");
        assert_eq!(
            content_type(Path::new("manifest.webmanifest")),
            "application/manifest+json"
        );
        assert_eq!(content_type(Path::new("icon-192.png")), "image/png");
        assert_eq!(content_type(Path::new("a.bin")), "application/octet-stream");
    }
}
