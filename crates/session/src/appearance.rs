//! A person's accent (`appearance/get`, `appearance/set`): the choice kept in their settings
//! ledger as `appearance.set`, and said to a client with the light and dark values to draw it in,
//! so no client works out a colour. The nearest choice wins, the person's over their
//! organization's over glave's plain look, and an organization that has everyone use its accent
//! narrows the person's choice to that (`specs/model.md`, "The look").

use glave_event::{Accent, AppearanceSet, Body, BrandSet};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::accent::shades;
use crate::protocol::{ApiError, OrganizationLook, Reply, code};
use crate::server::Server;
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `appearance/get`.
    pub(crate) async fn appearance(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let chosen = match self.setting::<AppearanceSet>(caller).await? {
            Some(Body::AppearanceSet(set)) => set.accent,
            _ => None,
        };
        Ok(reply(chosen, self.shown_brand(caller).await?))
    }

    /// Answers `appearance/set`: keeps `accent` as the caller's, unless their organization has
    /// everyone use its own.
    pub(crate) async fn set_appearance(
        &self,
        caller: &Caller,
        accent: Option<Accent>,
    ) -> Result<Reply, ApiError> {
        let brand = self.shown_brand(caller).await?;
        if brand.as_ref().is_some_and(|brand| brand.locked) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::AccentLocked));
        }
        self.keep_setting(caller, &AppearanceSet { accent }).await?;
        Ok(reply(accent, brand))
    }

    /// The caller's organization's brand, where it is shown to them: a person alone has none.
    async fn shown_brand(&self, caller: &Caller) -> Result<Option<BrandSet>, ApiError> {
        if self.organization_shown(caller).await? {
            self.brand_of(self.org_of(caller)).await
        } else {
            Ok(None)
        }
    }
}

/// The accent that holds: the organization's where everyone uses it, else the nearest chosen.
fn resolved(chosen: Option<Accent>, brand: Option<&BrandSet>) -> Accent {
    match brand {
        Some(brand) if brand.locked => brand.accent,
        Some(brand) => chosen.unwrap_or(brand.accent),
        None => chosen.unwrap_or(Accent::Plain),
    }
}

/// What `accent` is drawn as; none for glave's plain look.
fn drawn(accent: Accent) -> Option<crate::protocol::Shades> {
    match accent {
        Accent::Plain => None,
        Accent::Color { color, adjust } => Some(shades(color, adjust)),
    }
}

/// The reply that says `chosen`, what holds drawn, and what `brand` says.
fn reply(chosen: Option<Accent>, brand: Option<BrandSet>) -> Reply {
    Reply::Appearance {
        chosen,
        shades: drawn(resolved(chosen, brand.as_ref())),
        organization: brand.map(|brand| OrganizationLook::from(&brand)),
    }
}

#[cfg(test)]
mod tests {
    use glave_event::Color;

    use super::*;

    fn brand(accent: Accent, locked: bool) -> BrandSet {
        BrandSet {
            name: None,
            logo: None,
            accent,
            locked,
        }
    }

    #[test]
    fn the_nearest_choice_wins_and_a_lock_narrows_it_to_the_organization_s() {
        let own = Accent::Color {
            color: Color::rgb(1, 2, 3),
            adjust: false,
        };
        let theirs = Accent::Color {
            color: Color::rgb(4, 5, 6),
            adjust: false,
        };
        assert_eq!(resolved(None, None), Accent::Plain);
        assert_eq!(resolved(Some(own), None), own);
        assert_eq!(resolved(None, Some(&brand(theirs, false))), theirs);
        assert_eq!(resolved(Some(own), Some(&brand(theirs, false))), own);
        assert_eq!(
            resolved(Some(Accent::Plain), Some(&brand(theirs, false))),
            Accent::Plain
        );
        assert_eq!(resolved(Some(own), Some(&brand(theirs, true))), theirs);
    }
}
