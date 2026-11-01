//! Which of a route's targets may serve a request, and the request each one is sent: targets
//! that cannot honour it are skipped, and a conversation holding opaque reasoning stays with
//! the model that produced it whenever the route has it.

use std::borrow::Cow;
use std::fmt::Write as _;

use glave_event::{Opaque, Part};
use glave_ports::{Failure, Message, ModelRequest, ProviderError, Route};

use crate::estimate::Needs;
use crate::filtering::{self, Keeping, Kept};
use crate::target::Target;

/// Why a target was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skip {
    Tools,
    Images,
    Context {
        window: u64,
    },
    /// The person's filters keep the call from it.
    Filters(Kept),
}

/// A target that may serve a request, and what it asks of the router it is, if it is one.
pub(crate) type Candidate<'t> = (&'t Target, Option<Route>);

/// The targets that may serve `request`, in the route's order, each with what the person's
/// `filters` ask of it. With opaque reasoning in the conversation, only targets that accept
/// it, unless the route has none: then every capable target, and the reasoning is stripped for
/// whichever serves.
pub(crate) fn candidates<'t>(
    targets: &'t [Target],
    request: &ModelRequest,
    keeping: &Keeping,
    needs: &Needs<'_>,
) -> Result<Vec<Candidate<'t>>, ProviderError> {
    let mut capable = Vec::with_capacity(targets.len());
    let mut skipped = Vec::new();
    // A floor keeps every call to providers a router can vouch for, even with none among them.
    let routed = keeping.floor || targets.iter().any(|target| target.router.is_some());
    for target in targets {
        match skip(target, request, (keeping, routed), needs) {
            Ok(route) => capable.push((target, route)),
            Err(why) => skipped.push((target, why)),
        }
    }
    if capable.is_empty() {
        return Err(unserved(&skipped, &request.model, keeping));
    }
    if let Some(opaque) = needs.reasoning {
        let sticky: Vec<Candidate<'t>> = capable
            .iter()
            .filter(|(target, _)| accepts(target, opaque))
            .cloned()
            .collect();
        if !sticky.is_empty() {
            return Ok(sticky);
        }
    }
    Ok(capable)
}

/// What `target` asks of the router it is, if any, or why it is skipped.
fn skip(
    target: &Target,
    request: &ModelRequest,
    (keeping, routed): (&Keeping, bool),
    needs: &Needs<'_>,
) -> Result<Option<Route>, Skip> {
    if needs.tools && !target.can.tools {
        return Err(Skip::Tools);
    }
    if needs.images && !target.can.images {
        return Err(Skip::Images);
    }
    if let Some(window) = target.can.context_tokens {
        let wanted = needs
            .fit_tokens()
            .saturating_add(request.max_output_tokens.map_or(0, u64::from));
        if wanted > window {
            return Err(Skip::Context { window });
        }
    }
    if keeping.none_fit {
        return Err(Skip::Filters(Kept::NoneFits));
    }
    let Some(filters) = &keeping.filters else {
        return Ok(None);
    };
    filtering::route(filters, &request.model, target, routed).map_err(Skip::Filters)
}

/// The error when every target was skipped, for `model`: too long when every one was too
/// small, so the harness compacts, as it would after a provider's own refusal.
fn unserved(skipped: &[(&Target, Skip)], model: &str, keeping: &Keeping) -> ProviderError {
    let too_long = skipped
        .iter()
        .all(|(_, why)| matches!(why, Skip::Context { .. }));
    let mut reason = String::new();
    for (n, (target, why)) in skipped.iter().enumerate() {
        let sep = if n == 0 { "" } else { "; " };
        let name = &target.name;
        let _ = match why {
            Skip::Tools => write!(reason, "{sep}{name} cannot call tools"),
            Skip::Images => write!(reason, "{sep}{name} cannot read images"),
            Skip::Context { window } => write!(
                reason,
                "{sep}the conversation is longer than {name}'s context window of {window} tokens"
            ),
            Skip::Filters(Kept::NotARouter) if keeping.floor => write!(
                reason,
                "{sep}this server only uses providers it can check, and it can't check {name}"
            ),
            Skip::Filters(Kept::NotARouter) => {
                write!(
                    reason,
                    "{sep}{name} can't keep to your filters among providers"
                )
            }
            Skip::Filters(Kept::Unknown) if keeping.floor => write!(
                reason,
                "{sep}glave can't tell where {name}'s providers are based, so it can't keep to \
                 the countries this server allows"
            ),
            Skip::Filters(Kept::Unknown) => write!(
                reason,
                "{sep}glave can't tell where {name}'s providers are based, so it can't keep to the \
                 countries you chose"
            ),
            Skip::Filters(Kept::NoneFits) if keeping.floor && keeping.own => write!(
                reason,
                "{sep}no provider of {model} at {name} fits both your filters and what this \
                 server allows: change your filters or choose another model"
            ),
            Skip::Filters(Kept::NoneFits) if keeping.floor => write!(
                reason,
                "{sep}this server allows no provider of {model} at {name}: choose another model"
            ),
            Skip::Filters(Kept::NoneFits) => write!(
                reason,
                "{sep}no provider of {model} at {name} fits your filters: loosen them to use it"
            ),
        };
    }
    if reason.is_empty() {
        reason.push_str("the route has no targets");
    }
    ProviderError::Unserved {
        failure: if too_long && !skipped.is_empty() {
            Failure::TooLong
        } else {
            Failure::Rejected
        },
        reason,
    }
}

/// Whether `target` accepts `opaque` back: only the provider and model that produced it.
fn accepts(target: &Target, opaque: &Opaque) -> bool {
    target.can.reasoning && opaque.provider == target.provider && opaque.model == target.model
}

/// `request` as `target` is sent it: under the target's model name, without opaque reasoning
/// the target cannot accept, at the reasoning level [`Target::effort_for`] gives, and asking
/// what `route` does when the target is a router. True when reasoning was stripped. Borrowed
/// when nothing changes.
pub(crate) fn prepare<'r>(
    request: &'r ModelRequest,
    route: Option<&Route>,
    target: &Target,
    needs: &Needs<'_>,
) -> (Cow<'r, ModelRequest>, bool) {
    let foreign = needs.reasoning.is_some()
        && request.messages.iter().any(|message| match message {
            Message::Assistant { parts, .. } => parts.iter().any(|part| is_foreign(part, target)),
            _ => false,
        });
    let effort = target.effort_for(request.effort.as_deref());
    let unchanged = request.model == target.model && effort == request.effort.as_deref();
    if !foreign && unchanged && route == request.route.as_ref() {
        return (Cow::Borrowed(request), false);
    }
    let mut sent = request.clone();
    sent.model.clone_from(&target.model);
    sent.effort = effort.map(str::to_owned);
    sent.route = route.cloned();
    if foreign {
        for message in &mut sent.messages {
            if let Message::Assistant { parts, .. } = message {
                parts.retain(|part| !is_foreign(part, target));
            }
        }
    }
    (Cow::Owned(sent), foreign)
}

fn is_foreign(part: &Part, target: &Target) -> bool {
    matches!(part, Part::Reasoning { opaque: Some(opaque), .. } if !accepts(target, opaque))
}
