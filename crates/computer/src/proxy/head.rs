//! A request's head as a command's HTTP client sends it to a proxy: where it goes, and for
//! plain HTTP the head to send on in its place. Pure: bytes in, a request out.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::rules::Host;

/// The most a request's head may hold, its line and headers.
pub(crate) const MOST: usize = 16 * 1024;

/// A request the proxy was sent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Request {
    /// `CONNECT host:port`: a tunnel, answered once it is open.
    Tunnel { host: Host, port: u16 },
    /// Plain HTTP, sent on to the host as `head`: the target in origin form, the `Host` header
    /// the name decided on, and the connection closed after one response.
    Forward {
        host: Host,
        port: u16,
        head: Vec<u8>,
    },
}

/// Why a head could not be read as a request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// Not a request a proxy can take.
    Malformed,
    /// An absolute URL with a scheme other than `http`.
    Scheme,
}

/// Where a head ends: the index after its blank line, once `bytes` holds it.
pub(crate) fn end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at + 4)
}

/// The proxy's own credential as a request carries it: the password of `Proxy-Authorization:
/// Basic`, never shown.
#[derive(PartialEq, Eq)]
pub(crate) struct Credential(String);

impl Credential {
    /// What the request presented.
    pub(crate) fn presented(&self) -> &str {
        &self.0
    }

    /// The password `value`, a `Proxy-Authorization` header's, carries, if it is `Basic`.
    fn read(value: &str) -> Option<Self> {
        let (scheme, encoded) = value.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("basic") {
            return None;
        }
        let decoded = BASE64.decode(encoded.trim()).ok()?;
        let pair = String::from_utf8(decoded).ok()?;
        let (_, password) = pair.split_once(':')?;
        Some(Self(password.to_owned()))
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<hidden>)")
    }
}

/// Whether `line` holds a byte no head may: a control byte but a tab, which only a header's
/// value may hold. A bare line feed or carriage return is one, so a line cannot hide another.
fn controlled(line: &str, tab_ok: bool) -> bool {
    line.bytes()
        .any(|b| (b < 0x20 && !(tab_ok && b == b'\t')) || b == 0x7f)
}

/// The request `head` (up to and including its blank line) asks for, and the proxy credential
/// it presents. A head with a control byte, or a header folded onto a second line, is refused.
pub(crate) fn parse(head: &[u8]) -> Result<(Request, Option<Credential>), Unread> {
    let text = std::str::from_utf8(head).map_err(|_| Unread::Malformed)?;
    let mut lines = text.split("\r\n");
    let line = lines.next().ok_or(Unread::Malformed)?;
    if controlled(line, false) {
        return Err(Unread::Malformed);
    }
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Unread::Malformed);
    };
    if !version.starts_with("HTTP/1.") || method.is_empty() {
        return Err(Unread::Malformed);
    }
    let headers: Vec<(&str, &str)> = lines
        .take_while(|line| !line.is_empty())
        .map(header)
        .collect::<Result<_, _>>()?;
    let credential = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
        .and_then(|(_, value)| Credential::read(value));
    let request = request(method, target, version, &headers)?;
    Ok((request, credential))
}

/// One header line's name and value, trimmed; refused when folded (it starts with a space or
/// a tab), when its name is not one word, or when it holds a control byte.
fn header(line: &str) -> Result<(&str, &str), Unread> {
    let (name, value) = line.split_once(':').ok_or(Unread::Malformed)?;
    let word = !name.is_empty() && !name.contains([' ', '\t']);
    if !word || controlled(name, false) || controlled(value, true) {
        return Err(Unread::Malformed);
    }
    Ok((name, value.trim_matches([' ', '\t'])))
}

/// What a request line asks for, its headers beside it.
fn request(
    method: &str,
    target: &str,
    version: &str,
    headers: &[(&str, &str)],
) -> Result<Request, Unread> {
    if method == "CONNECT" {
        let (host, port) = authority(target, None)?;
        return Ok(Request::Tunnel { host, port });
    }
    let (authority_text, path) = match target.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => {
            let at = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let (authority, rest) = rest.split_at(at);
            // A fragment is the client's alone; a query with no path is the root's.
            let path = rest.split('#').next().unwrap_or_default();
            let path = if path.starts_with('/') {
                path.to_owned()
            } else {
                format!("/{path}")
            };
            (authority, path)
        }
        Some(_) => return Err(Unread::Scheme),
        None if target.starts_with('/') => {
            let named = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("host"))
                .map(|(_, value)| *value)
                .ok_or(Unread::Malformed)?;
            (named, target.to_owned())
        }
        None => return Err(Unread::Malformed),
    };
    let (host, port) = authority(authority_text, Some(80))?;
    let head = forwarded(method, &path, version, &host, port, headers);
    Ok(Request::Forward { host, port, head })
}

/// The host and port `text` names (`example.com:443`, `[::1]:8080`), the port `default` where
/// it names none. No user name: a proxy's client never sends one in earnest.
fn authority(text: &str, default: Option<u16>) -> Result<(Host, u16), Unread> {
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && (!host.contains(':') || host.ends_with(']')) => {
            let port = port.parse::<u16>().map_err(|_| Unread::Malformed)?;
            (host, port)
        }
        _ => (text, default.ok_or(Unread::Malformed)?),
    };
    let host = Host::parse(host).ok_or(Unread::Malformed)?;
    if port == 0 {
        return Err(Unread::Malformed);
    }
    Ok((host, port))
}

/// The head sent on for plain HTTP: the target in origin form, `Host` the name decided on, the
/// proxy's own headers dropped, and the connection closed after one response. Hygiene, not the
/// boundary: what bytes follow go to the address and port the proxy checked, whatever host
/// they name (`specs/sandbox.md`, "The outbound proxy").
fn forwarded(
    method: &str,
    path: &str,
    version: &str,
    host: &Host,
    port: u16,
    headers: &[(&str, &str)],
) -> Vec<u8> {
    let named = if port == 80 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    let mut out = format!("{method} {path} {version}\r\nHost: {named}\r\n");
    let dropped = [
        "host",
        "connection",
        "proxy-connection",
        "proxy-authorization",
        "keep-alive",
    ];
    for (name, value) in headers {
        if !dropped.iter().any(|d| name.eq_ignore_ascii_case(d)) {
            for part in [name, ": ", value, "\r\n"] {
                out.push_str(part);
            }
        }
    }
    out.push_str("Connection: close\r\n\r\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(head: &[u8]) -> Result<Request, Unread> {
        parse(head).map(|(request, _)| request)
    }

    fn name(text: &str) -> Host {
        Host::Name(text.into())
    }

    #[test]
    fn a_head_ends_at_its_blank_line() {
        assert_eq!(end(b"GET / HTTP/1.1\r\n\r\nrest"), Some(18));
        assert_eq!(end(b"GET / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn connect_names_a_host_and_a_port() {
        let tunnel = req(b"CONNECT Example.com:443 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(
            tunnel,
            Ok(Request::Tunnel {
                host: name("example.com"),
                port: 443
            })
        );
        let v6 = req(b"CONNECT [2606:4700::1111]:443 HTTP/1.1\r\n\r\n").unwrap();
        assert!(matches!(
            v6,
            Request::Tunnel {
                host: Host::Address(_),
                port: 443
            }
        ));
        for bad in [
            "CONNECT example.com HTTP/1.1\r\n\r\n",
            "CONNECT example.com:0 HTTP/1.1\r\n\r\n",
            "CONNECT example.com:99999 HTTP/1.1\r\n\r\n",
            "CONNECT ::1:443 HTTP/1.1\r\n\r\n",
            "CONNECT  example.com:443 HTTP/1.1\r\n\r\n",
            "CONNECT example.com:443 SPDY/3\r\n\r\n",
        ] {
            assert_eq!(req(bad.as_bytes()), Err(Unread::Malformed), "{bad}");
        }
    }

    #[test]
    fn plain_http_goes_on_in_origin_form_to_the_host_decided_on() {
        let head = b"GET http://example.com:8080/a?b HTTP/1.1\r\nHost: evil.example\r\n\
            Proxy-Connection: keep-alive\r\nProxy-Authorization: x\r\nAccept: */*\r\n\r\n";
        let Ok(Request::Forward { host, port, head }) = req(head) else {
            panic!("not forwarded");
        };
        assert_eq!((host, port), (name("example.com"), 8080));
        assert_eq!(
            String::from_utf8(head).unwrap(),
            "GET /a?b HTTP/1.1\r\nHost: example.com:8080\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
        let bare = req(b"GET http://example.com HTTP/1.1\r\n\r\n").unwrap();
        let Request::Forward { head, port: 80, .. } = bare else {
            panic!("not forwarded");
        };
        assert!(head.starts_with(b"GET / HTTP/1.1\r\nHost: example.com\r\n"));
    }

    #[test]
    fn origin_form_goes_to_its_host_header() {
        let head = b"POST /upload HTTP/1.1\r\nhost: example.com\r\nContent-Length: 3\r\n\r\n";
        let Ok(Request::Forward { host, port, .. }) = req(head) else {
            panic!("not forwarded");
        };
        assert_eq!((host, port), (name("example.com"), 80));
        assert_eq!(req(b"GET / HTTP/1.1\r\n\r\n"), Err(Unread::Malformed));
    }

    #[test]
    fn only_http_is_forwarded() {
        let https = req(b"GET https://example.com/ HTTP/1.1\r\n\r\n");
        assert_eq!(https, Err(Unread::Scheme));
        assert_eq!(
            req(b"GET example.com HTTP/1.1\r\n\r\n"),
            Err(Unread::Malformed)
        );
        assert_eq!(
            req(b"GET / HTTP/1.1\r\nno colon\r\n\r\n"),
            Err(Unread::Malformed)
        );
        assert_eq!(
            req(&[0xff, 0xfe, b'\r', b'\n', b'\r', b'\n']),
            Err(Unread::Malformed)
        );
    }

    #[test]
    fn a_head_with_a_control_byte_or_a_folded_header_is_refused() {
        for bad in [
            "GET http://a.example/ HTTP/1.1\r\nX: a\nHost: b.example\r\n\r\n",
            "GET http://a.example/ HTTP/1.1\r\nX: a\rHost: b.example\r\n\r\n",
            "GET http://a.example/\n HTTP/1.1\r\n\r\n",
            "GET http://a.example/ HTTP/1.1\r\nX: a\r\n Host: b.example\r\n\r\n",
            "GET http://a.example/ HTTP/1.1\r\nX: a\r\n\tmore\r\n\r\n",
            "GET http://a.example/ HTTP/1.1\r\nX : a\r\n\r\n",
            "GET http://a.example/ HTTP/1.1\r\nX: a\u{7f}\r\n\r\n",
            "GET http://a.example/ HTTP/1.1\r\nX: \0\r\n\r\n",
        ] {
            assert_eq!(req(bad.as_bytes()), Err(Unread::Malformed), "{bad:?}");
        }
        let tabbed = req(b"GET http://a.example/ HTTP/1.1\r\nX: a\tb\r\n\r\n").unwrap();
        let Request::Forward { head, .. } = tabbed else {
            panic!("not forwarded");
        };
        assert!(
            head.windows(6).any(|w| w == b"X: a\tb"),
            "a tab is kept in a value"
        );
    }

    #[test]
    fn the_scheme_is_any_case_and_a_query_or_fragment_ends_the_authority() {
        for (target, path) in [
            ("HTTP://a.example?x=1", "/?x=1"),
            ("http://a.example#top", "/"),
            ("http://a.example/p?q#f", "/p?q"),
        ] {
            let head = format!("GET {target} HTTP/1.1\r\n\r\n");
            let Ok(Request::Forward { host, head, .. }) = req(head.as_bytes()) else {
                panic!("{target} not forwarded");
            };
            assert_eq!(host, name("a.example"));
            let line = format!("GET {path} HTTP/1.1\r\n");
            assert!(head.starts_with(line.as_bytes()), "{target}");
        }
    }

    #[test]
    fn the_proxys_credential_is_the_basic_password_and_never_shown() {
        let encoded = BASE64.encode("t:a-token");
        let head = format!(
            "CONNECT a.example:443 HTTP/1.1\r\nProxy-Authorization: basic {encoded}\r\n\r\n"
        );
        let (_, credential) = parse(head.as_bytes()).unwrap();
        let credential = credential.unwrap();
        assert_eq!(credential.presented(), "a-token");
        assert_eq!(format!("{credential:?}"), "Credential(<hidden>)");
        for value in ["Bearer a-token", "Basic !!!", "Basic"] {
            let head =
                format!("CONNECT a.example:443 HTTP/1.1\r\nProxy-Authorization: {value}\r\n\r\n");
            assert_eq!(parse(head.as_bytes()).unwrap().1, None, "{value}");
        }
    }
}
