//! Signing in keeps nothing on the server until it is answered (`specs/session.md`, "Signing
//! in"): the browser holds its request, sealed, so a stranger asking to sign in however often
//! pushes no one's out, and each request is answered once.

use glave_event::OrgRole;
use glave_identity::testing::Authenticator;
use glave_session::protocol::{Request, code};
use glave_session::signin::LocalToken;
use serde_json::json;

use super::{invite, join, serve};
use crate::support::web::{ORIGIN, asked, elsewhere};
use crate::support::*;

/// Someone's sign-in, begun before a stranger asks to begin thousands, still finishes, and
/// only once: a server that kept 16 waiting requests forgot it.
#[test]
fn a_stranger_s_flood_of_sign_ins_never_pushes_out_someone_s_waiting_sign_in() {
    block_on!(async {
        let dir = folder("accounts-flood");
        let token = LocalToken::new([19; 32]);
        let served = serve(&dir, true, &token).await;
        let mut phone = Authenticator::new(ORIGIN).unwrap();
        let code = invite(&served, &token, OrgRole::Member).await;
        join(&served, code, &mut phone).await;
        let mut grace = elsewhere(&served, None);
        let options = asked(&mut grace, &Request::PasskeyGetOptions {}).await;
        let get = Request::PasskeyGet {
            ceremony: options["ceremony"].as_str().unwrap().to_owned(),
            credential: phone.get(&options["options"]).unwrap(),
        };
        let mut stranger = elsewhere(&served, None);
        for _ in 0..2_000 {
            let flood = asked(&mut stranger, &Request::PasskeyGetOptions {}).await;
            assert!(flood["ceremony"].is_string(), "{flood}");
        }
        let signed_in = asked(&mut grace, &get).await;
        assert!(signed_in["session"].is_string(), "{signed_in}");
        let again = asked(&mut grace, &get).await;
        assert_eq!(again, json!(code::NO_SIGN_IN), "answered once");
        served.close().await;
    });
}
