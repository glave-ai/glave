//! An organization's brand (`specs/model.md`, "The look"): its admins edit it, everyone in it
//! reads what it says of how glave looks, the nearest choice winning until it has everyone use
//! its accent, and a person alone sees none of it. A logo is a blob of the organization's, its
//! size and kind limited.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Accent, Auth, AuthMethod, Color, OrgRole, Sha256};
use glave_ports::{BlobStore, Caller, OrgId, UNREFERENCED_FOR_MS};
use glave_session::protocol::{ApiError, Logo, OrganizationLook, Reply, Request, Shades, code};
use glave_session::signin::person;

use crate::support::*;

const ISSUER: &str = "https://id.example.com";

/// Someone signed in through the identity provider as `subject`: a Member.
fn member(subject: &str) -> Caller {
    Caller {
        person: Some(person(ISSUER, subject).unwrap()),
        auth: Some(Auth {
            method: AuthMethod::Oidc,
            issuer: Some(ISSUER.to_owned()),
            subject: Some(subject.to_owned()),
            session: None,
        }),
        ..Caller::default()
    }
}

fn exact(hex: &str) -> Accent {
    Accent::Color {
        color: Color::new(hex).unwrap(),
        adjust: false,
    }
}

fn shades(hex: &str) -> Shades {
    let color = Color::new(hex).unwrap();
    Shades {
        light: color,
        dark: color,
    }
}

fn png() -> String {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0; 32]);
    STANDARD.encode(bytes)
}

fn set(name: Option<&str>, logo: Option<Sha256>, accent: Accent, locked: bool) -> Request {
    Request::BrandSet {
        name: name.map(str::to_owned),
        logo,
        accent,
        locked,
    }
}

async fn brand(server: &Tested<Shared>, caller: &Caller, request: Request) -> OrganizationLook {
    match server.request_as(caller, request).await {
        Ok(Reply::Brand { brand }) => brand,
        other => panic!("the brand, not {other:?}"),
    }
}

async fn upload(server: &Tested<Shared>, caller: &Caller) -> Sha256 {
    match server
        .request_as(caller, Request::BrandLogoUpload { data: png() })
        .await
    {
        Ok(Reply::LogoKept { sha256 }) => sha256,
        other => panic!("a logo kept, not {other:?}"),
    }
}

async fn seen(
    server: &Tested<Shared>,
    caller: &Caller,
) -> (Option<Accent>, Option<Shades>, Option<OrganizationLook>) {
    match server.request_as(caller, Request::AppearanceGet {}).await {
        Ok(Reply::Appearance {
            chosen,
            shades,
            organization,
        }) => (chosen, shades, organization),
        other => panic!("the appearance, not {other:?}"),
    }
}

async fn logo(
    server: &Tested<Shared>,
    caller: &Caller,
    request: Request,
) -> Result<Option<Logo>, ApiError> {
    match server.request_as(caller, request).await? {
        Reply::BrandLogo { logo } => Ok(logo),
        other => panic!("a logo, not {other:?}"),
    }
}

#[test]
fn everyone_reads_it_the_nearest_choice_winning_until_everyone_uses_its_accent() {
    let dir = folder("brand-accent");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let (owner, ada) = (Caller::default(), member("ada"));

        // Nothing kept yet: glave's plain look, and no name.
        let (chosen, drawn, organization) = seen(&server, &ada).await;
        assert_eq!((chosen, drawn), (None, None));
        assert_eq!(organization, None);

        let kept = brand(
            &server,
            &owner,
            set(Some("  Acme  "), None, exact("#1558d6"), false),
        )
        .await;
        assert_eq!(kept.name.as_deref(), Some("Acme"));

        let (_, drawn, organization) = seen(&server, &ada).await;
        assert_eq!(drawn, Some(shades("#1558d6")), "theirs, until she chooses");
        assert!(!organization.unwrap().locked);

        let own = Request::AppearanceSet {
            accent: Some(exact("#0f766e")),
        };
        assert!(server.request_as(&ada, own.clone()).await.is_ok());
        assert_eq!(
            seen(&server, &ada).await.1,
            Some(shades("#0f766e")),
            "hers, the nearest"
        );

        brand(
            &server,
            &owner,
            set(Some("Acme"), None, exact("#1558d6"), true),
        )
        .await;
        let (chosen, drawn, organization) = seen(&server, &ada).await;
        assert_eq!(
            drawn,
            Some(shades("#1558d6")),
            "the lock narrows her choice"
        );
        assert_eq!(
            chosen,
            Some(exact("#0f766e")),
            "and keeps it for when it opens"
        );
        assert!(organization.unwrap().locked);
        let refused = server.request_as(&ada, own).await.unwrap_err();
        assert_eq!(refused.code, code::NOT_ALLOWED);
        assert!(
            refused
                .message
                .to_string()
                .contains("everyone use its accent")
        );

        brand(
            &server,
            &owner,
            set(Some("Acme"), None, exact("#1558d6"), false),
        )
        .await;
        assert_eq!(
            seen(&server, &ada).await.1,
            Some(shades("#0f766e")),
            "hers again"
        );
    });
}

#[test]
fn only_its_admins_edit_it_and_a_name_has_a_limit() {
    let dir = folder("brand-admins");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let (owner, ada) = (Caller::default(), member("ada"));
        let edits = |who: Caller, name: &'static str| {
            let server = &server;
            async move {
                server
                    .request_as(&who, set(Some(name), None, Accent::Plain, false))
                    .await
                    .map(|_| ())
            }
        };
        let grace = member("grace");
        server
            .request_as(&grace, Request::AppearanceGet {})
            .await
            .unwrap();
        let admin = Request::RoleSet {
            person: person(ISSUER, "grace").unwrap(),
            role: OrgRole::Admin,
        };
        server.request_as(&owner, admin).await.unwrap();
        edits(grace.clone(), "Grace's")
            .await
            .expect("an Admin edits it");
        for request in [
            Request::BrandGet {},
            set(Some("Mine"), None, Accent::Plain, false),
            Request::BrandLogoUpload { data: png() },
        ] {
            let refused = server.request_as(&ada, request).await.unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED);
        }
        let long = "x".repeat(81);
        let refused = server
            .request_as(&owner, set(Some(&long), None, Accent::Plain, false))
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::INVALID_PARAMS);
    });
}

#[test]
fn a_person_alone_sees_none_of_it() {
    let dir = folder("brand-alone");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new())
            .build_on(&dir, Shared::default())
            .await;
        let alone = Caller::default();
        brand(
            &server,
            &alone,
            set(Some("Acme"), None, exact("#1558d6"), true),
        )
        .await;
        let (_, drawn, organization) = seen(&server, &alone).await;
        assert_eq!((drawn, organization), (None, None));
    });
}

#[test]
fn a_logo_is_held_while_named_and_limited_in_size_and_kind() {
    let dir = folder("brand-logo");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let (owner, ada) = (Caller::default(), member("ada"));
        assert_eq!(
            logo(&server, &ada, Request::BrandLogoGet {}).await.unwrap(),
            None
        );

        for (data, words) in [
            (STANDARD.encode(b"<svg></svg>"), "PNG, JPEG or WebP"),
            ("not base64!".to_owned(), ""),
        ] {
            let refused = server
                .request_as(&owner, Request::BrandLogoUpload { data })
                .await
                .unwrap_err();
            assert_eq!(refused.code, code::INVALID_PARAMS);
            assert!(refused.message.to_string().contains(words));
        }
        let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
        huge.resize(256 * 1024 + 1, 0);
        let refused = server
            .request_as(
                &owner,
                Request::BrandLogoUpload {
                    data: STANDARD.encode(huge),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code, code::LIMIT);

        let up = upload(&server, &owner).await;
        let named = set(None, Some(up.clone()), Accent::Plain, false);
        let kept = brand(&server, &owner, named).await;
        assert_eq!(kept.logo, Some(up.clone()));
        let read = logo(&server, &ada, Request::BrandLogoGet {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.data, png(), "for everyone in it");
        assert_eq!(seen(&server, &ada).await.2.unwrap().logo, Some(up.clone()));

        // Kept as it is, and gone with none named.
        brand(
            &server,
            &owner,
            set(Some("Acme"), Some(up), Accent::Plain, false),
        )
        .await;
        assert!(
            logo(&server, &ada, Request::BrandLogoGet {})
                .await
                .unwrap()
                .is_some()
        );
        brand(
            &server,
            &owner,
            set(Some("Acme"), None, Accent::Plain, false),
        )
        .await;
        assert_eq!(
            logo(&server, &ada, Request::BrandLogoGet {}).await.unwrap(),
            None
        );

        let never = Sha256::from_digest([7; 32]);
        let refused = server
            .request_as(&owner, set(None, Some(never), Accent::Plain, false))
            .await
            .unwrap_err();
        assert!(refused.message.to_string().contains("upload it again"));
    });
}

#[test]
fn its_name_is_what_it_is_called_in_a_listing() {
    let dir = folder("brand-listed");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let owner = Caller::default();
        brand(
            &server,
            &owner,
            set(Some("Acme"), None, Accent::Plain, false),
        )
        .await;
        let Reply::Organizations { organizations, .. } = server
            .request_as(&owner, Request::OrganizationList {})
            .await
            .unwrap()
        else {
            panic!("the organizations");
        };
        assert_eq!(organizations[0].name.to_string(), "Acme");
    });
}

#[test]
fn another_organization_s_brand_is_its_own() {
    let dir = folder("brand-orgs");
    block_on!(async {
        let log = Shared::default();
        let mut ours = Setup::new(Vec::new());
        ours.identity_provider = true;
        let (ours, _) = ours.build_on(&dir, log.clone()).await;
        let mut theirs = Setup::new(Vec::new());
        theirs.identity_provider = true;
        theirs.seed = 2;
        theirs.home = glave_ports::Scope::glaves_own(
            theirs_org(),
            glave_ports::ProjectId::from_uuid(uuid::Uuid::from_u128(42)),
        );
        let (theirs, _) = theirs.build_on(&dir, log).await;
        let owner = Caller::default();
        brand(
            &ours,
            &owner,
            set(Some("Acme"), None, exact("#1558d6"), true),
        )
        .await;
        let (_, drawn, organization) = seen(&theirs, &member("olga")).await;
        assert_eq!((drawn, organization), (None, None));
        let there = brand(&theirs, &owner, Request::BrandGet {}).await;
        assert_eq!(there.name, None);
    });
}

#[test]
fn a_logo_held_outlives_every_sweep_and_one_replaced_goes_at_the_next() {
    let dir = folder("brand-sweep");
    block_on!(async {
        let mut setup = Setup::new(Vec::new());
        setup.identity_provider = true;
        let (blobs, clock) = (setup.blobs.clone(), setup.clock.clone());
        let (server, _) = setup.build_on(&dir, Shared::default()).await;
        let owner = Caller::default();
        let org = scope().org();
        let up = |bytes: &'static [u8]| {
            let (server, owner) = (&server, &owner);
            async move {
                let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
                png.extend_from_slice(bytes);
                let data = STANDARD.encode(png);
                match server
                    .request_as(owner, Request::BrandLogoUpload { data })
                    .await
                {
                    Ok(Reply::LogoKept { sha256 }) => sha256,
                    other => panic!("a logo kept, not {other:?}"),
                }
            }
        };
        let (first, other) = (up(b"first").await, up(b"other").await);
        brand(
            &server,
            &owner,
            set(None, Some(first.clone()), Accent::Plain, false),
        )
        .await;
        clock.advance_millis(2 * UNREFERENCED_FOR_MS);
        // The upload no one named goes; the one held stays, however long.
        assert_eq!(blobs.sweep(org).await.unwrap().blobs, 1);
        assert!(
            logo(&server, &owner, Request::BrandLogoGet {})
                .await
                .unwrap()
                .is_some()
        );

        // Named again, it stays held; another named in its place, it goes and that one stays.
        brand(
            &server,
            &owner,
            set(Some("Acme"), Some(first.clone()), Accent::Plain, false),
        )
        .await;
        clock.advance_millis(2 * UNREFERENCED_FOR_MS);
        assert_eq!(blobs.sweep(org).await.unwrap().blobs, 0);
        let again = up(b"other").await;
        assert_eq!(again, other);
        brand(
            &server,
            &owner,
            set(Some("Acme"), Some(again.clone()), Accent::Plain, false),
        )
        .await;
        assert_eq!(
            blobs.sweep(org).await.unwrap().blobs,
            1,
            "the first, let go of"
        );
        let read = logo(&server, &owner, Request::BrandLogoGet {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.sha256, again);

        // None named, it goes too.
        brand(
            &server,
            &owner,
            set(Some("Acme"), None, Accent::Plain, false),
        )
        .await;
        assert_eq!(blobs.sweep(org).await.unwrap().blobs, 1);
        assert_eq!(
            logo(&server, &owner, Request::BrandLogoGet {})
                .await
                .unwrap(),
            None
        );
    });
}

fn theirs_org() -> OrgId {
    OrgId::from_uuid(uuid::Uuid::from_u128(41))
}
