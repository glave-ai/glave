//! A shared file, served at its link's own name under the share domain (`specs/session.md`,
//! "Sharing"; `specs/invariants.md`, 37): never at glave's own origin, and nothing of glave's
//! here, no app, no API, no sign-in. Its page is glave's own HTML with no script, under a
//! policy that runs none; the file itself is a picture or a PDF the page shows, or a download,
//! sandboxed should a browser open it. Every answer is kept nowhere (`no-store`), sends no
//! `Referer` and asks not to be indexed. A link that opens nothing is answered alike whatever
//! the reason: never made, stopped, or its project erased.

use glave_event::{AssetKind, ShareSet};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, read_all};
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request as HttpRequest, Response, StatusCode};
use std::net::IpAddr;

use super::body::Answer;

mod asking;
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// The most of a preview read back, in bytes: more than any preview holds (256 KB).
const MOST_PREVIEW: u64 = 1024 * 1024;

/// What a shared page may load: its own style and the file, nothing else, and no script.
const PAGE_POLICY: &str = "default-src 'none'; style-src 'self'; img-src 'self'; \
    frame-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// What the file itself may do where a browser opens it: nothing, sandboxed.
const FILE_POLICY: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; \
    sandbox; frame-ancestors 'self'";

/// A PDF framed by its page: Chrome's viewer shows none in a sandbox, so it is only kept to
/// frames of its own page.
const PDF_POLICY: &str = "frame-ancestors 'self'";

/// The page's style: plain, light or dark as the reader's system is.
const STYLE: &str = include_str!("shared.css");

/// Answers `request` to the share whose link's id is `id`, from the address `from`, its words
/// in the first language of its `Accept-Language` the server writes.
pub(super) async fn answer<P, L, B>(
    server: &Server<P, L, B>,
    id: &str,
    request: HttpRequest<Incoming>,
    from: IpAddr,
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let headers = request.headers();
    let language = language_of(headers);
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    let path = path.as_str();
    let posted = method == Method::POST;
    let head = method == Method::HEAD;
    if !posted && method != Method::GET && !head {
        let mut refused = answered(StatusCode::METHOD_NOT_ALLOWED, Answer::empty());
        refused
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return refused;
    }
    if path == "/style.css" {
        return typed(
            Bytes::from_static(STYLE.as_bytes()),
            "text/css; charset=utf-8",
            PAGE_POLICY,
        );
    }
    let found = match server.share_at(id).await {
        Ok(Some(found)) => found,
        Ok(None) => return gone(language),
        Err(error) => {
            tracing::warn!(error = %error.message, "could not read a share");
            return answered(StatusCode::SERVICE_UNAVAILABLE, Answer::empty());
        }
    };
    let (scope, share) = found;
    if let glave_event::Audience::People { .. } = share.audience {
        if posted {
            let asked = (from, language);
            return asking::posted(server, (scope, share), request, asked).await;
        }
        if let Some(token) = path.strip_prefix("/open/") {
            return asking::link_page(token, language);
        }
        let secure = server.shares_securely();
        let proof = asking::proof(request.headers(), secure).unwrap_or_default();
        let ledger = match server.share_ledger(&scope).await {
            Ok(ledger) => ledger,
            Err(error) => {
                tracing::warn!(error = %error.message, "could not read a share's proofs");
                return answered(StatusCode::SERVICE_UNAVAILABLE, Answer::empty());
            }
        };
        if !server.proven(&ledger, &share, &proof) {
            return if path == "/" {
                asking::ask(language)
            } else {
                answered(StatusCode::NOT_FOUND, Answer::empty())
            };
        }
    } else if posted {
        return answered(StatusCode::METHOD_NOT_ALLOWED, Answer::empty());
    }
    match path {
        "/" => page(server, &scope, &share, language).await,
        "/file" if shown(share.kind) => file(server, &scope, &share, (false, head)).await,
        "/download" => file(server, &scope, &share, (true, head)).await,
        _ => answered(StatusCode::NOT_FOUND, Answer::empty()),
    }
}

/// Whether a share's file is shown as itself on its page: a picture or a PDF.
fn shown(kind: AssetKind) -> bool {
    matches!(kind, AssetKind::Picture | AssetKind::Pdf)
}

/// The share's page: its name, the file shown as itself or its preview, Download, and who
/// shared it.
async fn page<P, L, B>(
    server: &Server<P, L, B>,
    scope: &Scope,
    share: &ShareSet,
    language: Language,
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let name = escaped(&share.name);
    let shown = match share.kind {
        AssetKind::Picture => format!("<img class=\"file\" src=\"/file\" alt=\"{name}\">"),
        AssetKind::Pdf => {
            format!("<iframe class=\"file\" src=\"/file\" title=\"{name}\"></iframe>")
        }
        _ => match &share.preview {
            Some(preview) => previewed(server, scope, preview, language).await,
            None => String::new(),
        },
    };
    let report = (server.inner.shares.as_ref())
        .and_then(|domain| domain.report.as_deref())
        .map(|to| {
            format!(
                " · <a href=\"mailto:{}?subject={}\">{}</a>",
                escaped(to),
                encoded(&format!("Report: {}", share.link)),
                escaped(&say(language, &Sentence::PageReport))
            )
        })
        .unwrap_or_default();
    let body = format!(
        "<main><h1>{name}</h1>{shown}<p><a class=\"button\" href=\"/download\">{download}</a></p></main>\
         <footer>{shared}{report}</footer>",
        download = escaped(&say(language, &Sentence::PageDownload)),
        shared = escaped(&say(language, &Sentence::PageShared)),
    );
    html(StatusCode::OK, &name, &body, language)
}

/// The share's preview, as it was kept, and what it leaves out; nothing when it can't be read.
async fn previewed<P, L, B>(
    server: &Server<P, L, B>,
    scope: &Scope,
    preview: &glave_event::MadePreview,
    language: Language,
) -> String
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let Some(bytes) = read(server, scope, &preview.blob.sha256, MOST_PREVIEW).await else {
        return String::new();
    };
    let markdown = escaped(&String::from_utf8_lossy(&bytes));
    let cut = if preview.cut {
        let said = escaped(&say(language, &Sentence::PreviewCut));
        format!("<p class=\"note\">{said}</p>")
    } else {
        String::new()
    };
    format!("<pre class=\"preview\">{markdown}</pre>{cut}")
}

/// The first language of `headers`'s `Accept-Language` the server writes, else English.
fn language_of(headers: &HeaderMap) -> Language {
    (headers.get(header::ACCEPT_LANGUAGE))
        .and_then(|value| value.to_str().ok())
        .and_then(Language::accepted)
        .unwrap_or(Language::English)
}

/// The page a link that opens nothing answers with.
fn gone(language: Language) -> Response<Answer> {
    let said = escaped(&say(language, &Sentence::LinkGone));
    html(
        StatusCode::NOT_FOUND,
        "Glave",
        &format!("<main><p>{said}</p></main>"),
        language,
    )
}

/// The share's file: shown as itself, or, as a download, saved under its name. Its bytes are
/// streamed as read; a share stopped while they go doesn't cut a download already under way,
/// which is at most 10 MB (a kept file's limit), so no piece checks the share again.
async fn file<P, L, B>(
    server: &Server<P, L, B>,
    scope: &Scope,
    share: &ShareSet,
    (download, head): (bool, bool),
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let blobs = server.inner.harness.blobs();
    let Ok(Some(found)) = blobs.get(scope, &share.blob.sha256).await else {
        return answered(StatusCode::NOT_FOUND, Answer::empty());
    };
    let policy = if !download && share.kind == AssetKind::Pdf {
        PDF_POLICY
    } else {
        FILE_POLICY
    };
    let media_type = if download {
        "application/octet-stream"
    } else {
        share.blob.media_type.as_str()
    };
    let length = found.reference.bytes;
    let body = if head {
        Answer::empty()
    } else {
        Answer::file(found.bytes, length)
    };
    let mut response = sent(body, media_type, policy);
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if download && let Ok(disposition) = HeaderValue::from_str(&disposition(&share.file_name)) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, disposition);
    }
    response
}

/// The blob `sha256` of `scope`, at most `limit` bytes; none when it can't be read.
async fn read<P, L, B>(
    server: &Server<P, L, B>,
    scope: &Scope,
    sha256: &glave_event::Sha256,
    limit: u64,
) -> Option<Vec<u8>>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let found = server
        .inner
        .harness
        .blobs()
        .get(scope, sha256)
        .await
        .ok()??;
    let mut bytes = found.bytes;
    read_all(bytes.as_mut(), limit).await.ok()
}

/// A whole page of glave's own, `title` and `body` already escaped.
fn html(status: StatusCode, title: &str, body: &str, language: Language) -> Response<Answer> {
    let (lang, dir) = match language {
        Language::RightToLeft => ("ar", "rtl"),
        Language::English | Language::Accented => ("en", "ltr"),
    };
    let page = format!(
        "<!doctype html><html lang=\"{lang}\" dir=\"{dir}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex, nofollow\"><title>{title}</title>\
         <link rel=\"stylesheet\" href=\"/style.css\"></head><body>{body}</body></html>"
    );
    let mut response = typed(Bytes::from(page), "text/html; charset=utf-8", PAGE_POLICY);
    *response.status_mut() = status;
    response
}

/// `bytes` as `media_type`, under `policy`, with what every answer here carries.
fn typed(bytes: Bytes, media_type: &str, policy: &'static str) -> Response<Answer> {
    sent(Answer::bytes(bytes), media_type, policy)
}

/// `body` as `media_type`, under `policy`, with what every answer here carries.
fn sent(body: Answer, media_type: &str, policy: &'static str) -> Response<Answer> {
    let mut response = answered(StatusCode::OK, body);
    let headers = response.headers_mut();
    if let Ok(media_type) = HeaderValue::from_str(media_type) {
        headers.insert(header::CONTENT_TYPE, media_type);
    }
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(policy),
    );
    response
}

/// An answer with `status` and what every answer here carries: kept nowhere, no `Referer`,
/// never indexed, never sniffed.
fn answered(status: StatusCode, body: Answer) -> Response<Answer> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-robots-tag"),
        HeaderValue::from_static("noindex, nofollow"),
    );
    response
}

/// `text` safe in HTML's text and its quoted attributes.
fn escaped(text: &str) -> String {
    let mut written = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => written.push_str("&amp;"),
            '<' => written.push_str("&lt;"),
            '>' => written.push_str("&gt;"),
            '"' => written.push_str("&quot;"),
            '\'' => written.push_str("&#39;"),
            _ => written.push(c),
        }
    }
    written
}

/// `text` percent-encoded for a URL's query, every byte but letters, digits and `-._~`.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// `Content-Disposition` saving a download as `name`: plainly where it is ASCII, and as
/// RFC 8187's UTF-8 always.
fn disposition(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "attachment; filename=\"{plain}\"; filename*=UTF-8''{}",
        encoded(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_page_writes_is_text_and_never_markup() {
        assert_eq!(
            escaped("<script>\"a\" & 'b'</script>"),
            "&lt;script&gt;&quot;a&quot; &amp; &#39;b&#39;&lt;/script&gt;"
        );
    }

    #[test]
    fn a_download_is_saved_under_its_name_in_any_script() {
        assert_eq!(
            disposition("Q3 report.docx"),
            "attachment; filename=\"Q3 report.docx\"; filename*=UTF-8''Q3%20report.docx"
        );
        assert_eq!(
            disposition("räkna\".xlsx"),
            "attachment; filename=\"r_kna_.xlsx\"; filename*=UTF-8''r%C3%A4kna%22.xlsx"
        );
    }
}
