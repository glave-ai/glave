//! A share for specific people, asking who opens it (`specs/session.md`, "Sharing"): with no
//! proof, its page asks for an email and sends a code there only if the share lists it,
//! answering alike either way; the right code gives the browser a proof for this share's name
//! alone, in a cookie no page script can read, good for thirty days. Forms are posted as HTML
//! posts them, with no script.

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

/// The cookie a proof is kept in, on this share's name alone.
const PROOF: &str = "glave_share";

/// The most a form's body holds, in bytes: an address and a code.
const MOST_FORM: usize = 4 * 1024;

/// How long a form's body may take to arrive.
const FORM_WITHIN: Duration = Duration::from_secs(30);

/// The proof a request's cookies hold for this share's name, if any.
pub(super) fn proof(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .find_map(|cookie| {
            let (name, value) = cookie.trim().split_once('=')?;
            (name == PROOF && !value.is_empty()).then(|| value.to_owned())
        })
}

/// The page that asks for an email, sending a code to it.
pub(super) fn ask(language: Language) -> Response<Answer> {
    let said = |sentence: Sentence| escaped(&say(language, &sentence));
    let body = format!(
        "<main><h1>{heading}</h1><p>{ask}</p>\
         <form method=\"post\" action=\"/code\"><label>{email}\
         <input type=\"email\" name=\"email\" required autocomplete=\"email\"></label>\
         <button class=\"button\">{send}</button></form></main>",
        heading = said(Sentence::PageSomeone),
        ask = said(Sentence::PageAskEmail),
        email = said(Sentence::PageEmail),
        send = said(Sentence::PageSendCode),
    );
    html(
        StatusCode::OK,
        &said(Sentence::PageSomeone),
        &body,
        language,
    )
}

/// The page that asks for the code sent to `email`, with `note` above it.
fn ask_code(email: &str, note: &str, language: Language) -> Response<Answer> {
    let said = |sentence: Sentence| escaped(&say(language, &sentence));
    let body = format!(
        "<main><h1>{heading}</h1><p>{note}</p>\
         <form method=\"post\" action=\"/open\"><input type=\"hidden\" name=\"email\" \
         value=\"{email}\"><label>{code}<input name=\"code\" required \
         autocomplete=\"one-time-code\" autocapitalize=\"characters\"></label>\
         <button class=\"button\">{open}</button></form><p><a href=\"/\">{again}</a></p></main>",
        heading = said(Sentence::PageSomeone),
        note = escaped(note),
        email = escaped(email),
        code = said(Sentence::PageCode),
        open = said(Sentence::PageOpen),
        again = said(Sentence::PageAgain),
    );
    html(
        StatusCode::OK,
        &said(Sentence::PageSomeone),
        &body,
        language,
    )
}

/// Answers a form posted to a share for specific people: `/code` sends a code to the address it
/// names, if the share lists it; `/open` trades the code for a proof, then sends the browser
/// back to the share's page, which opens now.
pub(super) async fn posted<P, L, B>(
    server: &Server<P, L, B>,
    (scope, share): (&Scope, &ShareSet),
    request: HttpRequest<Incoming>,
    from: IpAddr,
    language: Language,
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
    let email = field(&form, "email").unwrap_or_default();
    match path.as_str() {
        "/code" => match server.send_code(scope, share, &email, from, language).await {
            Ok(()) => ask_code(&email, &say(language, &Sentence::PageCodeSent), language),
            Err(mut refused) => {
                refused.message.translate(language);
                ask_code(&email, &refused.message.to_string(), language)
            }
        },
        "/open" => {
            let code = field(&form, "code").unwrap_or_default();
            match server.open_with_code(scope, share, &email, &code).await {
                Ok(Some(proof)) => opened(&proof, server.shares_securely()),
                Ok(None) => ask_code(&email, &say(language, &Sentence::PageCodeWrong), language),
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
    let secure = if secure { "; Secure" } else { "" };
    let max_age = crate::shares::PROOF_FOR.as_secs();
    let cookie =
        format!("{PROOF}={proof}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}");
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
        let form = "email=ada%40example.com&code=abcd+efgh&other=1";
        assert_eq!(field(form, "email").as_deref(), Some("ada@example.com"));
        assert_eq!(field(form, "code").as_deref(), Some("abcd efgh"));
        assert_eq!(field(form, "missing"), None);
        assert_eq!(field("email=%zz", "email"), None);
    }

    #[test]
    fn a_proof_is_read_from_its_cookie_alone() {
        let mut headers = HeaderMap::new();
        let cookies = HeaderValue::from_static("other=1; glave_share=abc; x=y");
        headers.insert(header::COOKIE, cookies);
        assert_eq!(proof(&headers).as_deref(), Some("abc"));
        assert_eq!(proof(&HeaderMap::new()), None);
    }
}
