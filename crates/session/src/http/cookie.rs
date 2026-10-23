//! A browser's session over HTTP, handed over in two halves (`specs/one-server.md`, "Finding
//! it"), so that neither alone signs anyone in. The first is a cookie the page's scripts cannot
//! read (`HttpOnly`), sent with no other site's requests (`SameSite=Strict`), and only over
//! HTTPS where the server is reached over HTTPS (`Secure`). The second is the answer's
//! `session`, which the app keeps in its origin's storage and presents as its bearer. A browser
//! sends a cookie to every port of its host, so a page another program serves on this computer
//! gets the first half; storage is its origin's alone, port included, so no such page reads the
//! second. A script injected into the app reads the second but never the first. The cookie is
//! named after its server, so two glaves on one computer keep theirs apart; it lasts 30 days
//! from the last request that presented it, as the session does, and signing out clears it.

use glave_identity::signin::{SESSION_PREFIX, SignInError};
use hyper::header::{self, HeaderMap, HeaderValue};

use crate::protocol::{ApiError, Reply};

/// How long the cookie lasts from the answer that last set it, in seconds: the 30 days a
/// session lasts from its last use.
const LIFETIME: u64 = 30 * 24 * 60 * 60;

/// One server's cookie, as one request presented it.
pub(crate) struct Jar {
    /// The cookie's name: `glave-` and the server's id.
    name: String,
    /// Whether the browser reached the server over HTTPS, so the cookie travels only there.
    secure: bool,
    /// The cookie's half of a session, as the request presented it.
    presented: Option<String>,
}

/// What a request's answer does with the cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keeping {
    /// The request presented no session of a browser.
    Nothing,
    /// A browser signed in with its session: the cookie is set again, for 30 more days.
    Renew,
    /// The browser signed out: the cookie is cleared.
    Clear,
}

impl Jar {
    /// The cookie of the server whose id is `server`, as `headers` present it, for a browser
    /// that reached it over HTTPS when `secure` says so.
    pub(crate) fn new(server: &str, secure: bool, headers: &HeaderMap) -> Self {
        let name = format!("glave-{server}");
        let presented = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(named, _)| *named == name)
            .map(|(_, value)| value.to_owned());
        Self {
            name,
            secure,
            presented,
        }
    }

    /// The token `bearer` stands for: a browser session's second half joined with the first,
    /// from the cookie; any other token as it is. A browser's session without its cookie signs
    /// no one in.
    pub(crate) fn token(&self, bearer: &str) -> Result<String, SignInError> {
        match (bearer.strip_prefix(SESSION_PREFIX), &self.presented) {
            (Some(second), Some(first)) => Ok(format!("{SESSION_PREFIX}{first}{second}")),
            (Some(_), None) => Err(SignInError::EndedSession),
            (None, _) => Ok(bearer.to_owned()),
        }
    }

    /// `reply` as the browser is handed it, and the cookie to set with it: a new session's
    /// first half in the cookie and its second in the answer; else what `keeping` says.
    pub(crate) fn hand(
        &self,
        reply: Result<Reply, ApiError>,
        keeping: Keeping,
    ) -> (Result<Reply, ApiError>, Option<HeaderValue>) {
        if let Ok(Reply::Browser { session, server }) = &reply
            && let Some((first, second)) = split(session.expose())
        {
            let cookie = self.cookie(first, LIFETIME);
            let session = glave_event::Secret::new(second);
            let server = server.clone();
            return (Ok(Reply::Browser { session, server }), cookie);
        }
        let cookie = match keeping {
            Keeping::Clear if reply.is_ok() => self.cookie("", 0),
            _ => self.renewed(keeping),
        };
        (reply, cookie)
    }

    /// The cookie set again for 30 more days, when `keeping` renews it.
    pub(crate) fn renewed(&self, keeping: Keeping) -> Option<HeaderValue> {
        match (keeping, &self.presented) {
            (Keeping::Renew, Some(first)) => self.cookie(first, LIFETIME),
            _ => None,
        }
    }

    /// The `Set-Cookie` value that keeps `value` for `seconds`.
    fn cookie(&self, value: &str, seconds: u64) -> Option<HeaderValue> {
        let secure = if self.secure { "; Secure" } else { "" };
        let cookie = format!(
            "{}={value}; Path=/; Max-Age={seconds}; HttpOnly; SameSite=Strict{secure}",
            self.name
        );
        HeaderValue::from_str(&cookie).ok()
    }
}

/// A session's `token` in its two halves: the first half of its random digits, for the cookie,
/// and the second, after the session prefix, for the page.
fn split(token: &str) -> Option<(&str, String)> {
    let digits = token.strip_prefix(SESSION_PREFIX)?;
    let (first, second) = digits.split_at_checked(digits.len() / 2)?;
    Some((first, format!("{SESSION_PREFIX}{second}")))
}

#[cfg(test)]
mod tests {
    use glave_event::Secret;

    use super::*;

    const SERVER: &str = "0192f5c6-0000-7000-8000-0000000005e7";

    fn presenting(cookie: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = cookie {
            headers.insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        }
        headers
    }

    fn browser(session: &str) -> Reply {
        Reply::Browser {
            session: Secret::new(session),
            server: SERVER.into(),
        }
    }

    #[test]
    fn a_new_session_is_handed_over_in_halves_that_join_again() {
        let token = format!("{SESSION_PREFIX}{}{}", "a".repeat(32), "b".repeat(32));
        let jar = Jar::new(SERVER, false, &HeaderMap::new());
        let (reply, cookie) = jar.hand(Ok(browser(&token)), Keeping::Nothing);
        let Ok(Reply::Browser { session, server }) = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(server, SERVER);
        assert_eq!(
            session.expose(),
            format!("{SESSION_PREFIX}{}", "b".repeat(32))
        );
        let cookie = cookie.unwrap();
        let cookie = cookie.to_str().unwrap();
        assert_eq!(
            cookie,
            format!(
                "glave-{SERVER}={}; Path=/; Max-Age=2592000; HttpOnly; SameSite=Strict",
                "a".repeat(32)
            )
        );

        let first = cookie.split_once(';').unwrap().0;
        let back = Jar::new(
            SERVER,
            false,
            &presenting(Some(&format!("other=1; {first}"))),
        );
        assert_eq!(back.token(session.expose()).unwrap(), token, "joined again");
        let alone = Jar::new(SERVER, false, &HeaderMap::new());
        assert!(
            alone.token(session.expose()).is_err(),
            "the page's half alone"
        );
        assert!(alone.token(&token).is_err(), "nor the whole in one piece");
        let another = Jar::new("another", false, &presenting(Some(first)));
        assert!(
            another.token(session.expose()).is_err(),
            "another glave's cookie"
        );
    }

    #[test]
    fn over_https_the_cookie_travels_only_there() {
        let token = format!("{SESSION_PREFIX}{}", "c".repeat(64));
        let (_, cookie) =
            Jar::new(SERVER, true, &HeaderMap::new()).hand(Ok(browser(&token)), Keeping::Nothing);
        assert!(cookie.unwrap().to_str().unwrap().ends_with("; Secure"));
    }

    #[test]
    fn signing_out_clears_the_cookie_and_signed_in_requests_renew_it() {
        let jar = Jar::new(
            SERVER,
            false,
            &presenting(Some(&format!("glave-{SERVER}=abcd"))),
        );
        let done = || Ok(Reply::Done {});
        let (_, cleared) = jar.hand(done(), Keeping::Clear);
        assert!(
            cleared
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("glave-{SERVER}=; Path=/; Max-Age=0;"))
        );
        let refused = Err(ApiError::new(crate::protocol::code::INVALID_PARAMS, "no"));
        assert_eq!(jar.hand(refused, Keeping::Clear).1, None, "not signed out");
        let (_, renewed) = jar.hand(done(), Keeping::Renew);
        assert!(
            renewed
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("glave-{SERVER}=abcd; Path=/; Max-Age=2592000;"))
        );
        assert_eq!(jar.hand(done(), Keeping::Nothing).1, None);
        let none = Jar::new(SERVER, false, &HeaderMap::new());
        assert_eq!(none.renewed(Keeping::Renew), None, "no cookie to renew");
        assert_eq!(none.token("glave_local_abc").unwrap(), "glave_local_abc");
    }
}
