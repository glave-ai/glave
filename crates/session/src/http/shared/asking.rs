//! A share for specific people, asking who opens it (`specs/session.md`, "Sharing"): with no
//! proof, its page asks for an email, naming nothing of the file, and glave emails a link there
//! only if the share lists it, answering alike either way. The link's page asks for one click,
//! a form posted, so a mail scanner that only fetches it spends nothing; the click gives the
//! browser a proof for this share's name alone, in a cookie no page script can read (`__Host-`
//! over HTTPS), good for thirty days. Forms are posted as HTML posts them, with no script.

use std::net::IpAddr;
use std::time::Duration;

use glave_event::ShareSet;
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Request as HttpRequest, Response, StatusCode};

use super::{answered, escaped, html};
use crate::http::body::Answer;
use crate::server::Server;
use crate::words::{Language, Sentence, say};

/// The cookie a proof is kept in, on this share's name alone: over HTTPS with the `__Host-`
/// prefix, which a browser takes only from that name, `Secure`, at `/`.
fn cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-glave_share"
    } else {
        "glave_share"
    }
}

/// The most a form's body holds, in bytes: an address or a link's token.
const MOST_FORM: usize = 4 * 1024;

/// How long a form's body may take to arrive.
const FORM_WITHIN: Duration = Duration::from_secs(30);

/// The proof a request's cookies hold for this share's name, if any.
pub(super) fn proof(headers: &HeaderMap, secure: bool) -> Option<String> {
    let wanted = cookie_name(secure);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .find_map(|cookie| {
            let (name, value) = cookie.trim().split_once('=')?;
            (name == wanted && !value.is_empty()).then(|| value.to_owned())
        })
}

/// A page of a share for people, its heading saying only that someone shared something, with
/// `inner` under it.
fn asking(inner: &str, language: Language) -> Response<Answer> {
    let heading = escaped(&say(language, &Sentence::PageSomeone));
    let body = format!("<main><h1>{heading}</h1>{inner}</main>");
    html(StatusCode::OK, &heading, &body, language)
}

/// The page that asks for an email, `note` above it, emailing a link there.
fn ask_with(note: &str, language: Language) -> Response<Answer> {
    let said = |sentence: Sentence| escaped(&say(language, &sentence));
    let inner = format!(
        "<p>{note}</p><form method=\"post\" action=\"/link\"><label>{email}\
         <input type=\"email\" name=\"email\" required autocomplete=\"email\"></label>\
         <button class=\"button\">{send}</button></form>",
        note = escaped(note),
        email = said(Sentence::PageEmail),
        send = said(Sentence::PageSendLink),
    );
    asking(&inner, language)
}

/// The page that asks for an email, emailing a link there.
pub(super) fn ask(language: Language) -> Response<Answer> {
    ask_with(&say(language, &Sentence::PageAskEmail), language)
}

/// The page an emailed link opens: one click, a form posted, opens the share.
pub(super) fn link_page(token: &str, language: Language) -> Response<Answer> {
    let said = |sentence: Sentence| escaped(&say(language, &sentence));
    let inner = format!(
        "<form method=\"post\" action=\"/open\"><input type=\"hidden\" name=\"token\" \
         value=\"{token}\"><button class=\"button\">{open}</button></form>",
        token = escaped(token),
        open = said(Sentence::PageOpen),
    );
    asking(&inner, language)
}

/// Answers a form posted to a share for specific people: `/link` emails a link to the address
/// it names, if the share lists it; `/open` trades a link's token for a proof, then sends the
/// browser back to the share's page, which opens now.
pub(super) async fn posted<P, L, B>(
    server: &Server<P, L, B>,
    (scope, share): (Scope, ShareSet),
    request: HttpRequest<Incoming>,
    (from, language): (IpAddr, Language),
) -> Response<Answer>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    let path = request.uri().path().to_owned();
    let read = tokio::time::timeout(
        FORM_WITHIN,
        crate::http::read(request.into_body(), MOST_FORM),
    );
    let Ok(Ok(form)) = read.await else {
        return answered(StatusCode::BAD_REQUEST, Answer::empty());
    };
    match path.as_str() {
        "/link" => {
            let email = field(&form, "email").unwrap_or_default();
            match server.ask_link(scope, share, email, (from, language)).await {
                Ok(()) => ask_with(&say(language, &Sentence::PageLinkSent), language),
                Err(mut refused) => {
                    refused.message.translate(language);
                    ask_with(&refused.message.to_string(), language)
                }
            }
        }
        "/open" => {
            let token = field(&form, "token").unwrap_or_default();
            match server.open_with_link(&scope, &share, &token).await {
                Ok(Some(proof)) => opened(&proof, server.shares_securely()),
                Ok(None) => ask_with(&say(language, &Sentence::PageLinkWrong), language),
                Err(error) => {
                    tracing::warn!(error = %error.message, "could not open a share");
                    answered(StatusCode::SERVICE_UNAVAILABLE, Answer::empty())
                }
            }
        }
        _ => answered(StatusCode::NOT_FOUND, Answer::empty()),
    }
}

/// Sends the browser back to the share's page, holding `proof` for this share's name alone.
fn opened(proof: &str, secure: bool) -> Response<Answer> {
    let mut response = answered(StatusCode::SEE_OTHER, Answer::empty());
    let headers = response.headers_mut();
    headers.insert(header::LOCATION, HeaderValue::from_static("/"));
    let max_age = crate::shares::PROOF_FOR.as_secs();
    let secured = if secure { "; Secure" } else { "" };
    let name = cookie_name(secure);
    let cookie =
        format!("{name}={proof}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secured}");
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        headers.insert(header::SET_COOKIE, cookie);
    }
    response
}

/// The value of `name` in a form's body (`application/x-www-form-urlencoded`), decoded.
fn field(form: &str, name: &str) -> Option<String> {
    form.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (decoded(key)? == name).then(|| decoded(value)).flatten()
    })
}

/// `text` as a form writes it decoded: `+` a space, `%XX` the byte it names.
fn decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = char::from(rest.next()?).to_digit(16)?;
                let low = char::from(rest.next()?).to_digit(16)?;
                bytes.push(u8::try_from(high * 16 + low).ok()?);
            }
            _ => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_s_fields_are_read_as_a_browser_writes_them() {
        let form = "email=ada%40example.com&token=ab+cd&other=1";
        assert_eq!(field(form, "email").as_deref(), Some("ada@example.com"));
        assert_eq!(field(form, "token").as_deref(), Some("ab cd"));
        assert_eq!(field(form, "missing"), None);
        assert_eq!(field("email=%zz", "email"), None);
    }

    #[test]
    fn a_proof_is_read_from_its_cookie_alone_by_its_name_for_the_scheme() {
        let mut headers = HeaderMap::new();
        let cookies = HeaderValue::from_static("other=1; glave_share=abc; __Host-glave_share=def");
        headers.insert(header::COOKIE, cookies);
        assert_eq!(proof(&headers, false).as_deref(), Some("abc"));
        assert_eq!(proof(&headers, true).as_deref(), Some("def"));
        assert_eq!(proof(&HeaderMap::new(), true), None);
    }
}
