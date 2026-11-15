//! The provider a router last served a thread's conversation with at each target, as its
//! `model.responded` events say (`upstream`), so every later call of the conversation asks the
//! router for that one first and its prompt cache keeps hitting (`specs/gateway.md`,
//! "Fallback"). Only the calls that send the conversation count: the turn's own, a subagent's
//! and a compaction's summary, never the judge's or a title's, which send prompts of their own.

use std::collections::BTreeMap;

use glave_event::{Body, Event, ModelResponded};
use glave_ports::Purpose;

use crate::harness::Thread;

/// The purposes whose calls send a thread's conversation, and so share its prefix.
const CONVERSATION: [Purpose; 3] = [Purpose::Main, Purpose::Subagent, Purpose::Summary];

/// Each target's latest provider, by the target's name, as the router named it.
pub(crate) type Upstreams = BTreeMap<String, String>;

/// What `events`, a thread's log, say served its conversation last at each target.
pub(crate) fn upstreams_in(events: &[Event]) -> Upstreams {
    let mut upstreams = Upstreams::new();
    for event in events {
        if let Body::ModelResponded(responded) = &event.body {
            see(&mut upstreams, responded);
        }
    }
    upstreams
}

/// Takes `responded` into `upstreams`, when a router served the conversation's call.
fn see(upstreams: &mut Upstreams, responded: &ModelResponded) {
    let conversation = (responded.route.as_deref())
        .is_some_and(|route| CONVERSATION.iter().any(|purpose| purpose.id() == route));
    let (Some(target), Some(upstream)) = (&responded.target, &responded.upstream) else {
        return;
    };
    if conversation {
        upstreams.insert(target.clone(), upstream.clone());
    }
}

impl Thread {
    /// Takes in who served a call of the thread just recorded as `responded`.
    pub(crate) fn served(&mut self, responded: &ModelResponded) {
        see(&mut self.upstreams, responded);
    }
}
