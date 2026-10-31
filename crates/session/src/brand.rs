//! An organization's brand (`specs/model.md`, "The look"): its name, logo and accent, and whether
//! everyone uses that accent, kept in a ledger of its own in the organization's scope, as a screen's
//! save has no file to go to on a server with many organizations. Its admins edit it; everyone in
//! it reads what it says of how glave looks. The logo is a blob of the organization's, its size
//! and kind limited, each kept under a holder of its own, let go of once another is written down.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{Accent, Body, BrandSet, HeldLogo, Payload, Sha256, ThreadId, decode};
use glave_harness::kind_of;
use glave_ports::stream::Reserved;
use glave_ports::{
    BlobError, BlobStore, Caller, EventLog, ModelProvider, OrgId, Pieces, Scope, read_all,
};

use crate::people::Permit;
use crate::protocol::{ApiError, Logo, OrganizationLook, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

/// The largest a logo may be, in bytes.
const MOST_LOGO: u64 = 256 * 1024;
/// The kinds a logo may be: pictures a browser draws and that run nothing.
const LOGO_KINDS: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];
/// The longest an organization's name may be, in characters.
const LONGEST_NAME: usize = 80;

impl From<&BrandSet> for OrganizationLook {
    fn from(brand: &BrandSet) -> Self {
        Self {
            name: brand.name.clone(),
            logo: brand.logo.as_ref().map(|logo| logo.blob.sha256.clone()),
            accent: brand.accent,
            locked: brand.locked,
        }
    }
}

/// The reply that says `brand`.
fn reply(brand: &BrandSet) -> Reply {
    Reply::Brand {
        brand: brand.into(),
    }
}

/// A brand that says nothing: glave's plain look, and anyone's own choice.
const fn none() -> BrandSet {
    BrandSet {
        name: None,
        logo: None,
        accent: Accent::Plain,
        locked: false,
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// What `org`'s brand ledger last says; none until an admin keeps one, or when this glave
    /// cannot read it.
    pub(crate) async fn brand_of(&self, org: OrgId) -> Result<Option<BrandSet>, ApiError> {
        let latest = self
            .inner
            .harness
            .log()
            .latest_of(
                &Scope::of_organization(org),
                Reserved::Brand.id(),
                &[BrandSet::TYPE],
            )
            .await?;
        Ok(latest
            .and_then(|raw| decode(raw).ok())
            .and_then(|event| match event.body {
                Body::BrandSet(brand) => Some(brand),
                _ => None,
            }))
    }

    /// Answers `brand/get`.
    pub(crate) async fn brand(&self, caller: &Caller) -> Result<Reply, ApiError> {
        self.permitted(caller, Permit::Brand).await?;
        let brand = self.brand_of(self.org_of(caller)).await?;
        Ok(reply(&brand.unwrap_or_else(none)))
    }

    /// Answers `brand/set`: a logo it does not hold yet it holds from now on, under a holder of
    /// its own, and only once that is written down lets go of the one before. Failing anywhere
    /// leaves one too many, for the next save to let go of if that was the last written.
    pub(crate) async fn set_brand(
        &self,
        caller: &Caller,
        name: Option<String>,
        logo: Option<Sha256>,
        accent: Accent,
        locked: bool,
    ) -> Result<Reply, ApiError> {
        self.permitted(caller, Permit::Brand).await?;
        let name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        if name
            .as_ref()
            .is_some_and(|name| name.chars().count() > LONGEST_NAME)
        {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::BrandName { most: LONGEST_NAME },
            ));
        }
        let org = self.org_of(caller);
        let scope = Scope::of_organization(org);
        let before = self.brand_of(org).await?.and_then(|brand| brand.logo);
        let same = |sha256: &Sha256| {
            before
                .as_ref()
                .is_some_and(|held| held.blob.sha256 == *sha256)
        };
        let logo = match logo {
            Some(sha256) if same(&sha256) => before.clone(),
            Some(sha256) => Some(self.hold_logo(&scope, &sha256).await?),
            None => None,
        };
        let brand = BrandSet {
            name,
            logo,
            accent,
            locked,
        };
        self.keep_latest(&scope, Reserved::Brand.id(), &[BrandSet::TYPE], &brand)
            .await?;
        if let Some(before) = before.filter(|before| Some(before) != brand.logo.as_ref()) {
            self.inner
                .harness
                .blobs()
                .release(&scope, before.holder)
                .await
                .map_err(|error| ApiError::internal(&error))?;
        }
        Ok(reply(&brand))
    }

    /// Answers `brand/logo/upload`: keeps a logo for `brand/set` to name.
    pub(crate) async fn upload_logo(&self, caller: &Caller, data: &str) -> Result<Reply, ApiError> {
        self.permitted(caller, Permit::Brand).await?;
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| ApiError::new(code::INVALID_PARAMS, Sentence::FileUnreadable))?;
        let kind = kind_of("", &bytes)
            .filter(|kind| LOGO_KINDS.contains(&kind.as_str()))
            .ok_or_else(|| ApiError::new(code::INVALID_PARAMS, Sentence::LogoKind))?;
        let scope = Scope::of_organization(self.org_of(caller));
        let kept = self
            .inner
            .harness
            .blobs()
            .put(
                &scope,
                None,
                &kind,
                MOST_LOGO,
                Box::new(Pieces::from(bytes)),
            )
            .await
            .map_err(|error| match error {
                BlobError::TooLarge(_) => ApiError::new(
                    code::LIMIT,
                    Sentence::LogoTooLarge {
                        kilobytes: MOST_LOGO / 1024,
                    },
                ),
                BlobError::Storage(_) => ApiError::internal(&error),
            })?;
        Ok(Reply::LogoKept {
            sha256: kept.sha256,
        })
    }

    /// Answers `brand/logo/get`: the logo of the caller's organization, for everyone in it.
    pub(crate) async fn logo(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let org = self.org_of(caller);
        let Some(HeldLogo { blob: logo, .. }) =
            self.brand_of(org).await?.and_then(|brand| brand.logo)
        else {
            return Ok(Reply::BrandLogo { logo: None });
        };
        let blobs = self.inner.harness.blobs();
        let found = blobs
            .get(&Scope::of_organization(org), &logo.sha256)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        let Some(mut blob) = found else {
            return Ok(Reply::BrandLogo { logo: None });
        };
        let bytes = read_all(&mut *blob.bytes, MOST_LOGO)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        Ok(Reply::BrandLogo {
            logo: Some(Logo {
                sha256: logo.sha256,
                media_type: logo.media_type,
                data: STANDARD.encode(bytes),
            }),
        })
    }

    /// Holds the logo kept under `sha256` under a holder of its own; refused when it is not
    /// there any more.
    async fn hold_logo(&self, scope: &Scope, sha256: &Sha256) -> Result<HeldLogo, ApiError> {
        let harness = &self.inner.harness;
        let blobs = harness.blobs();
        let found = blobs
            .get(scope, sha256)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        let gone = || ApiError::new(code::INVALID_PARAMS, Sentence::LogoGone);
        let blob = found.ok_or_else(gone)?.reference;
        let holder = ThreadId::new(harness.clock().now(), harness.entropy().bytes()?)?;
        let held = blobs
            .refer(scope, sha256, holder)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        if held {
            Ok(HeldLogo { blob, holder })
        } else {
            Err(gone())
        }
    }
}
