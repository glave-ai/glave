//! A request's head as a command's HTTP client sends it to a proxy: where it goes, and for
//! plain HTTP the head to send on in its place. Pure: bytes in, a request out.

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

/// The request `head` (up to and including its blank line) asks for.
pub(crate) fn parse(head: &[u8]) -> Result<Request, Unread> {
    let text = std::str::from_utf8(head).map_err(|_| Unread::Malformed)?;
    let mut lines = text.split("\r\n");
    let line = lines.next().ok_or(Unread::Malformed)?;
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
        .map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim(), value.trim()))
                .ok_or(Unread::Malformed)
        })
        .collect::<Result<_, _>>()?;
    if method == "CONNECT" {
        let (host, port) = authority(target, None)?;
        return Ok(Request::Tunnel { host, port });
    }
    let (authority_text, path) = match target.split_once("://") {
        Some(("http", rest)) => rest.find('/').map_or((rest, "/"), |at| rest.split_at(at)),
        Some(_) => return Err(Unread::Scheme),
        None if target.starts_with('/') => {
            let named = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("host"))
                .map(|(_, value)| *value)
                .ok_or(Unread::Malformed)?;
            (named, target)
        }
        None => return Err(Unread::Malformed),
    };
    let (host, port) = authority(authority_text, Some(80))?;
    let head = forwarded(method, path, version, &host, port, &headers);
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
/// proxy's own headers dropped, and the connection closed after one response, so a second
/// request on it cannot reach another host unchecked.
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
        let tunnel = parse(b"CONNECT Example.com:443 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(
            tunnel,
            Ok(Request::Tunnel {
                host: name("example.com"),
                port: 443
            })
        );
        let v6 = parse(b"CONNECT [2606:4700::1111]:443 HTTP/1.1\r\n\r\n").unwrap();
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
            assert_eq!(parse(bad.as_bytes()), Err(Unread::Malformed), "{bad}");
        }
    }

    #[test]
    fn plain_http_goes_on_in_origin_form_to_the_host_decided_on() {
        let head = b"GET http://example.com:8080/a?b HTTP/1.1\r\nHost: evil.example\r\n\
            Proxy-Connection: keep-alive\r\nProxy-Authorization: x\r\nAccept: */*\r\n\r\n";
        let Ok(Request::Forward { host, port, head }) = parse(head) else {
            panic!("not forwarded");
        };
        assert_eq!((host, port), (name("example.com"), 8080));
        assert_eq!(
            String::from_utf8(head).unwrap(),
            "GET /a?b HTTP/1.1\r\nHost: example.com:8080\r\nAccept: */*\r\nConnection: close\r\n\r\n"
        );
        let bare = parse(b"GET http://example.com HTTP/1.1\r\n\r\n").unwrap();
        let Request::Forward { head, port: 80, .. } = bare else {
            panic!("not forwarded");
        };
        assert!(head.starts_with(b"GET / HTTP/1.1\r\nHost: example.com\r\n"));
    }

    #[test]
    fn origin_form_goes_to_its_host_header() {
        let head = b"POST /upload HTTP/1.1\r\nhost: example.com\r\nContent-Length: 3\r\n\r\n";
        let Ok(Request::Forward { host, port, .. }) = parse(head) else {
            panic!("not forwarded");
        };
        assert_eq!((host, port), (name("example.com"), 80));
        assert_eq!(parse(b"GET / HTTP/1.1\r\n\r\n"), Err(Unread::Malformed));
    }

    #[test]
    fn only_http_is_forwarded() {
        let https = parse(b"GET https://example.com/ HTTP/1.1\r\n\r\n");
        assert_eq!(https, Err(Unread::Scheme));
        assert_eq!(
            parse(b"GET example.com HTTP/1.1\r\n\r\n"),
            Err(Unread::Malformed)
        );
        assert_eq!(
            parse(b"GET / HTTP/1.1\r\nno colon\r\n\r\n"),
            Err(Unread::Malformed)
        );
        assert_eq!(
            parse(&[0xff, 0xfe, b'\r', b'\n', b'\r', b'\n']),
            Err(Unread::Malformed)
        );
    }
}
