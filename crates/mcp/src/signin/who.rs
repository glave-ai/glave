//! Who a sign-in is, where the authorization server says it in what it granted: an OpenID
//! Connect ID token's claims (`id_token`, when the person granted `openid` anyway), or a
//! workspace the token response names, as Notion's does. Nothing more is asked for, and nothing
//! is fetched. The ID token came straight from the token endpoint over TLS, so its claims are
//! read without checking its signature, as OpenID Connect allows there (Core 3.1.3.7); they
//! only name the account to the person who signed in, never decide anything.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_event::seen;
use serde_json::{Map, Value};

/// The longest name kept for who a sign-in is, in characters: anything longer is cut there.
const LONGEST: usize = 80;

/// Who the token response `fields` says signed in: the organization or workspace first, which
/// tells one business account from another, then the person by their name, user name or email;
/// none when it says nothing.
pub(super) fn signed_in_as(fields: &Value) -> Option<String> {
    let claims = fields
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(claims)
        .unwrap_or_default();
    let named = |map: &Map<String, Value>, key: &str| {
        map.get(key)
            .and_then(Value::as_str)
            .map(tidy)
            .filter(|it| !it.is_empty())
    };
    let response = fields.as_object().cloned().unwrap_or_default();
    named(&response, "workspace_name")
        .or_else(|| named(&claims, "org_name"))
        .or_else(|| named(&claims, "name"))
        .or_else(|| named(&claims, "preferred_username"))
        .or_else(|| named(&claims, "nickname"))
        .or_else(|| named(&claims, "email"))
}

/// The claims of the JWT `token`: its middle part, base64url JSON; none for anything else.
fn claims(token: &str) -> Option<Map<String, Value>> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    match serde_json::from_slice(&bytes).ok()? {
        Value::Object(claims) => Some(claims),
        _ => None,
    }
}

/// `text` on one line, its runs of space one, with only the letters a person sees (no control
/// or format character: no bidirectional override or isolate, no zero-width mark, so it can't
/// dress itself as another name), at most [`LONGEST`].
fn tidy(text: &str) -> String {
    let shown: String = text
        .chars()
        .filter(|it| it.is_whitespace() || seen(*it))
        .collect();
    let words: Vec<&str> = shown.split_whitespace().collect();
    words.join(" ").chars().take(LONGEST).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn token(claims: &Value) -> String {
        let part = |value: &Value| URL_SAFE_NO_PAD.encode(value.to_string());
        format!("{}.{}.", part(&json!({"alg": "none"})), part(claims))
    }

    #[test]
    fn says_the_workspace_then_the_person() {
        let person = token(&json!({"sub": "1", "name": "Ada  Lovelace\n", "email": "a@x.test"}));
        assert_eq!(
            signed_in_as(&json!({"id_token": person})).as_deref(),
            Some("Ada Lovelace")
        );
        let both = json!({"id_token": person, "workspace_name": "Acme Corp"});
        assert_eq!(signed_in_as(&both).as_deref(), Some("Acme Corp"));
        let org = token(&json!({"org_name": "Glave Inc.", "name": "Ada"}));
        assert_eq!(
            signed_in_as(&json!({"id_token": org})).as_deref(),
            Some("Glave Inc.")
        );
        let mail = token(&json!({"email": "ada@example.test"}));
        assert_eq!(
            signed_in_as(&json!({"id_token": mail})).as_deref(),
            Some("ada@example.test")
        );
    }

    #[test]
    fn says_nothing_it_was_not_told() {
        assert_eq!(signed_in_as(&json!({"access_token": "t"})), None);
        assert_eq!(signed_in_as(&json!({"id_token": "not a token"})), None);
        assert_eq!(
            signed_in_as(&json!({"id_token": token(&json!(["a"]))})),
            None
        );
        let blank = token(&json!({"name": "  ", "preferred_username": "ada"}));
        assert_eq!(
            signed_in_as(&json!({"id_token": blank})).as_deref(),
            Some("ada")
        );
        let turned = token(&json!({"name": "Ada\u{202E}ecalevoL\u{200B} \u{2066}Corp\u{2069}"}));
        assert_eq!(
            signed_in_as(&json!({"id_token": turned})).as_deref(),
            Some("AdaecalevoL Corp"),
            "no override, isolate or zero-width mark"
        );
        let long = "x".repeat(200);
        let cut = signed_in_as(&json!({"workspace_name": long})).unwrap_or_default();
        assert_eq!(cut.chars().count(), LONGEST);
    }
}
