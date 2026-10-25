//! Which of a route's targets may serve a request, and the request each one is sent: targets
//! that cannot honour it are skipped, and a conversation holding opaque reasoning stays with
//! the model that produced it whenever the route has it.

use std::borrow::Cow;
use std::fmt::Write as _;

use glave_event::{Opaque, Part};
use glave_ports::{Failure, Message, ModelRequest, ProviderError};

use crate::estimate::Needs;
use crate::target::Target;

/// Why a target was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skip {
    Tools,
    Images,
    Context { window: u64 },
}

/// The targets that may serve `request`, in the route's order. With opaque reasoning in the
/// conversation, only targets that accept it, unless the route has none: then every capable
/// target, and the reasoning is stripped for whichever serves.
pub(crate) fn candidates<'t>(
    targets: &'t [Target],
    request: &ModelRequest,
    needs: &Needs<'_>,
) -> Result<Vec<&'t Target>, ProviderError> {
    let mut capable = Vec::with_capacity(targets.len());
    let mut skipped = Vec::new();
    for target in targets {
        match skip(target, request, needs) {
            None => capable.push(target),
            Some(why) => skipped.push((target, why)),
        }
    }
    if capable.is_empty() {
        return Err(unserved(&skipped));
    }
    if let Some(opaque) = needs.reasoning {
        let sticky: Vec<&Target> = capable
            .iter()
            .copied()
            .filter(|target| accepts(target, opaque))
            .collect();
        if !sticky.is_empty() {
            return Ok(sticky);
        }
    }
    Ok(capable)
}

fn skip(target: &Target, request: &ModelRequest, needs: &Needs<'_>) -> Option<Skip> {
    if needs.tools && !target.can.tools {
        return Some(Skip::Tools);
    }
    if needs.images && !target.can.images {
        return Some(Skip::Images);
    }
    let window = target.can.context_tokens?;
    let wanted = needs
        .fit_tokens()
        .saturating_add(request.max_output_tokens.map_or(0, u64::from));
    (wanted > window).then_some(Skip::Context { window })
}

/// The error when every target was skipped: too long when every one was too small, so the
/// harness compacts, as it would after a provider's own refusal.
fn unserved(skipped: &[(&Target, Skip)]) -> ProviderError {
    let too_long = skipped
        .iter()
        .all(|(_, why)| matches!(why, Skip::Context { .. }));
    let mut reason = String::new();
    for (n, (target, why)) in skipped.iter().enumerate() {
        let sep = if n == 0 { "" } else { "; " };
        let _ = match why {
            Skip::Tools => write!(reason, "{sep}{} cannot call tools", target.name),
            Skip::Images => write!(reason, "{sep}{} cannot read images", target.name),
            Skip::Context { window } => write!(
                reason,
                "{sep}the conversation is longer than {}'s context window of {window} tokens",
                target.name
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
/// the target cannot accept, and at the reasoning level [`Target::effort_for`] gives. True
/// when reasoning was stripped. Borrowed when nothing changes.
pub(crate) fn prepare<'r>(
    request: &'r ModelRequest,
    target: &Target,
    needs: &Needs<'_>,
) -> (Cow<'r, ModelRequest>, bool) {
    let foreign = needs.reasoning.is_some()
        && request.messages.iter().any(|message| match message {
            Message::Assistant { parts, .. } => parts.iter().any(|part| is_foreign(part, target)),
            _ => false,
        });
    let effort = target.effort_for(request.effort.as_deref());
    if !foreign && request.model == target.model && effort == request.effort.as_deref() {
        return (Cow::Borrowed(request), false);
    }
    let mut sent = request.clone();
    sent.model.clone_from(&target.model);
    sent.effort = effort.map(str::to_owned);
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
