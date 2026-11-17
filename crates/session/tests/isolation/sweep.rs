//! The sweep: every person calls every method of the table with every other person's ids, and
//! each answer must be the one an id nobody has gets, with the ids put back as placeholders,
//! and must name nothing of anyone else's; so does glave acting on its own for each person but
//! the one on a server's computer, as a schedule's run does, which is also refused all that only
//! that person may do. What each person sees of their own is the same before the sweep and
//! after it.

use std::collections::BTreeSet;

use glave_event::{Auth, AuthMethod};
use glave_ports::{Caller, EventLog};
use glave_session::protocol::Request;
use serde_json::{Map, Value, json};

use super::table::{Kind, placeholders, table};
use super::world::{World, ask, request};

/// What one call found: which method, who called with whose ids, and what each answer was.
pub(super) type Finding = String;

/// Calls every template that names nothing of anyone's, as each person: what they answer
/// must name nothing of anyone else's.
pub(super) async fn own_calls<L: EventLog + Clone + 'static>(world: &World<L>) -> Vec<Finding> {
    let mut found = Vec::new();
    for (index, person) in world.people.iter().enumerate() {
        let org = &world.orgs[person.server];
        let others = ids_of_others(world, index);
        for (method, templates) in table() {
            for template in templates.iter().filter(|t| placeholders(t).is_empty()) {
                let answer = ask(org, &person.caller, request(method, template.clone())).await;
                let shown = answer.to_string();
                if let Some(id) = others.iter().find(|id| shown.contains(id.as_str())) {
                    found.push(format!(
                        "{method} {template} by {}: names {id} of another's: {shown}",
                        person.name
                    ));
                }
            }
        }
    }
    found
}

/// Every id but those of `person`, and the people of their own organization, whom its admins
/// see.
fn ids_of_others<L: EventLog + 'static>(world: &World<L>, person: usize) -> BTreeSet<String> {
    let org = world.people.get(person).map(|it| it.org);
    world
        .people
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != person)
        .flat_map(|(_, other)| {
            other
                .ids
                .iter()
                .filter(move |(kind, _)| {
                    !matches!(kind, Kind::Person | Kind::Invite) || Some(other.org) != org
                })
                .flat_map(|(_, ids)| ids)
        })
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}

/// One way to fill a template: for each placeholder, the victim's id or the caller's own.
struct Variant {
    params: Map<String, Value>,
    /// The same with every victim's id replaced by one nobody has.
    nobodys: Map<String, Value>,
    /// Each id filled in, the victim's and nobody's, with the placeholder it stands for.
    filled: Vec<(String, String, &'static str)>,
}

/// The variants of `template` when `attacker` calls with `victim`'s ids: every choice of
/// placeholders the victim's (at least one), the rest the attacker's own (or nobody's, when
/// they have none), each victim's id of that kind in turn.
fn variants(
    template: &Value,
    attacker: &super::world::Person,
    victim: &super::world::Person,
) -> Vec<Variant> {
    let holes = placeholders(template);
    let mut out = Vec::new();
    for mask in 1..(1u32 << holes.len()) {
        let mut partial = vec![Variant {
            params: template.as_object().cloned().unwrap_or_default(),
            nobodys: template.as_object().cloned().unwrap_or_default(),
            filled: Vec::new(),
        }];
        for (bit, (param, kind)) in holes.iter().enumerate() {
            let theirs = mask & (1 << bit) != 0;
            let choices: Vec<Value> = if theirs {
                victim.ids.get(kind).cloned().unwrap_or_default()
            } else {
                // The caller's own, or one nobody has when they hold none of the kind.
                vec![
                    attacker
                        .own(*kind)
                        .cloned()
                        .unwrap_or_else(|| kind.nobodys()),
                ]
            };
            let mut next = Vec::new();
            for variant in &partial {
                for choice in &choices {
                    let mut params = variant.params.clone();
                    let mut nobodys = variant.nobodys.clone();
                    let mut filled = variant.filled.clone();
                    params.insert(param.clone(), choice.clone());
                    if theirs {
                        let nobody = kind.nobodys();
                        filled.push((text(choice), text(&nobody), kind.placeholder()));
                        nobodys.insert(param.clone(), nobody);
                    } else {
                        nobodys.insert(param.clone(), choice.clone());
                    }
                    next.push(Variant {
                        params,
                        nobodys,
                        filled,
                    });
                }
            }
            partial = next;
        }
        out.extend(partial);
    }
    out
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// `params` as a request, and the caller making it: one presenting a browser's session when
/// the template says `@caller`.
fn split(method: &str, mut params: Map<String, Value>, caller: &Caller) -> (Request, Caller) {
    let mut caller = caller.clone();
    if let Some(session) = params.remove("@caller") {
        let mut auth = caller.auth.clone().unwrap_or(Auth {
            method: AuthMethod::Local,
            issuer: None,
            subject: None,
            session: None,
        });
        auth.session = Some(text(&session));
        caller.auth = Some(auth);
    }
    (request(method, Value::Object(params)), caller)
}

/// `answer` with each filled id put back as its placeholder.
fn normalized(answer: &Value, filled: &[(String, String, &'static str)], nobody: bool) -> String {
    let mut shown = answer.to_string();
    for (theirs, nobodys, placeholder) in filled {
        let id = if nobody { nobodys } else { theirs };
        shown = shown.replace(id.as_str(), placeholder);
    }
    shown
}

/// Every person calls every method with every other person's ids.
pub(super) async fn sweep<L: EventLog + Clone + 'static>(world: &World<L>) -> Vec<Finding> {
    sweep_by(world, |person| Some(person.caller.clone())).await
}

/// What only the person on a server's computer may do, the session's `local` check: folders
/// there, web search, keeping it awake, credits, answering what a folder asks to be trusted,
/// gone folders, model accounts, connections, secrets, the memory switch and sign-up links.
const LOCAL_ONLY: &[&str] = &[
    "folder/list",
    "search/set",
    "awake/set",
    "credits/list",
    "credits/add",
    "trust/answer",
    "project/folder/set",
    "project/remove",
    "provider/connect",
    "provider/list",
    "provider/refresh",
    "provider/remove",
    "connection/start",
    "connection/list",
    "connection/remove",
    "secret/add",
    "secret/list",
    "secret/remove",
    "memory/set",
    "signup/link",
];

/// `person` as glave acts for them on its own, as a schedule's run does: signed in by no one
/// (`on_behalf`), in their organization; none for the person on a server's computer, whom glave
/// acts as as they call.
fn on_behalf_of(person: &super::world::Person) -> Option<Caller> {
    let who = person.caller.person?;
    let org = person.caller.org.or_else(|| {
        let n = u128::try_from(person.org).ok()? + 1;
        Some(glave_ports::OrgId::from_uuid(uuid::Uuid::from_u128(n)))
    });
    Some(Caller {
        person: Some(who),
        auth: Some(Auth {
            method: AuthMethod::OnBehalf,
            issuer: None,
            subject: None,
            session: None,
        }),
        org,
        ..Caller::default()
    })
}

/// Everyone but the person on a server's computer, as glave acts for them on its own: they
/// reach no one else's, as the sweep finds of everyone, and each method only the person on the
/// computer may use refuses them, as it refuses them signed in from elsewhere.
pub(super) async fn on_behalf<L: EventLog + Clone + 'static>(world: &World<L>) -> Vec<Finding> {
    let mut found = sweep_by(world, on_behalf_of).await;
    let methods: BTreeSet<&str> = table().iter().map(|(method, _)| *method).collect();
    for method in LOCAL_ONLY
        .iter()
        .filter(|method| !methods.contains(*method))
    {
        found.push(format!(
            "{method}, only the person on the computer's, is no method"
        ));
    }
    for person in &world.people {
        let Some(behalf) = on_behalf_of(person) else {
            continue;
        };
        let org = &world.orgs[person.server];
        let gated = table().into_iter().filter(|(m, _)| LOCAL_ONLY.contains(m));
        for (method, templates) in gated {
            for template in templates {
                let mut params = template.as_object().cloned().unwrap_or_default();
                for (param, kind) in placeholders(&template) {
                    let own = person.own(kind).cloned();
                    params.insert(param, own.unwrap_or_else(|| kind.nobodys()));
                }
                let (theirs, as_them) = split(method, params.clone(), &behalf);
                let theirs = ask(org, &as_them, theirs).await;
                let (signed, as_signed) = split(method, params, &person.caller);
                let signed = ask(org, &as_signed, signed).await;
                let refused = theirs.get("error").is_some();
                if !refused || (signed.get("error").is_some() && theirs != signed) {
                    found.push(format!(
                        "{method} {template} for {} on their behalf, not refused as signed in \
                         from elsewhere:\n  on their behalf: {theirs}\n  signed in: {signed}",
                        person.name
                    ));
                }
            }
        }
    }
    found
}

/// The sweep, each person calling as `calls` makes them, or not at all.
async fn sweep_by<L: EventLog + Clone + 'static>(
    world: &World<L>,
    calls: impl Fn(&super::world::Person) -> Option<Caller>,
) -> Vec<Finding> {
    let mut found = Vec::new();
    for (a, attacker) in world.people.iter().enumerate() {
        let Some(calling) = calls(attacker) else {
            continue;
        };
        let org = &world.orgs[attacker.server];
        let others = ids_of_others(world, a);
        for victim in world.people.iter().filter(|v| v.name != attacker.name) {
            for (method, templates) in table() {
                for template in &templates {
                    let holes = placeholders(template);
                    let named = |kinds: &[Kind]| holes.iter().any(|(_, k)| kinds.contains(k));
                    // A link's code signs in the person on its server's computer, whoever
                    // holds it there; only another server's is someone else's. A person or an
                    // invite is their organization's admins' to see; only another
                    // organization's is someone else's. An invite's code is its holder's to
                    // join with: in any organization for someone with a glave account, who may
                    // belong to several, and in their own for anyone a company login signs in.
                    let joins = named(&[Kind::InviteCode])
                        && method == "invite/accept"
                        && with_account(&calling);
                    if (named(&[Kind::Code]) && victim.server == attacker.server)
                        || (named(&[Kind::Person, Kind::Invite, Kind::InviteCode])
                            && victim.org == attacker.org)
                        || joins
                    {
                        continue;
                    }
                    // What an invite's code opens is its holder's to see, so its answer differs
                    // from a code's that opens nothing; it still names no one else's ids.
                    let shown_to_holder = named(&[Kind::InviteCode]) && method == "invite/show";
                    for variant in variants(template, attacker, victim) {
                        let (theirs, caller) = split(method, variant.params, &calling);
                        let (nobodys, _) = split(method, variant.nobodys, &calling);
                        let theirs = ask(org, &caller, theirs).await;
                        let nobodys = ask(org, &caller, nobodys).await;
                        let shown = normalized(&theirs, &variant.filled, false);
                        let expected = normalized(&nobodys, &variant.filled, true);
                        let named = others.iter().find(|id| shown.contains(id.as_str()));
                        if (shown != expected && !shown_to_holder) || named.is_some() {
                            found.push(format!(
                                "{method} {template} by {} ({:?}) with {}'s ids {:?}:\n  \
                                 theirs: {shown}\n  nobody's: {expected}",
                                attacker.name,
                                calling.auth.as_ref().map(|auth| &auth.method),
                                victim.name,
                                variant.filled.iter().map(|f| &f.0).collect::<Vec<_>>(),
                            ));
                        }
                    }
                }
            }
        }
    }
    found
}

/// What each person sees of their own things, by person.
pub(super) async fn snapshot<L: EventLog + Clone + 'static>(world: &World<L>) -> Vec<Value> {
    let mut seen = Vec::new();
    for person in &world.people {
        let org = &world.orgs[person.server];
        let mut mine = Vec::new();
        let mut call = async |method: &str, params: Value| {
            let answer = ask(org, &person.caller, request(method, params.clone())).await;
            mine.push(json!({"method": method, "params": params, "answer": answer}));
        };
        for thread in person.ids.get(&Kind::Thread).into_iter().flatten() {
            call("thread/get", json!({"thread": thread})).await;
            call("thread/events", json!({"thread": thread})).await;
            call("thread/sources", json!({"thread": thread})).await;
            call("thread/changes", json!({"thread": thread})).await;
            call("thread/steps", json!({"thread": thread})).await;
            call("thread/plan", json!({"thread": thread})).await;
        }
        for project in person.ids.get(&Kind::Project).into_iter().flatten() {
            call("project/get", json!({"project": project})).await;
            call("thread/list", json!({"project": project})).await;
            call("memory/list", json!({"project": project})).await;
        }
        for method in [
            "project/list",
            "schedule/list",
            "tasks/list",
            "provider/list",
            "credential/list",
            "secret/list",
            "connection/list",
            "connector/list",
            "search/get",
            "memory/list",
            "push/get",
            "library/list",
        ] {
            call(method, json!({})).await;
        }
        if let Some(token) = &person.token {
            let verified = org
                .browsers
                .verify(token, true)
                .await
                .map_err(|e| e.to_string());
            mine.push(json!({"browser": format!("{verified:?}")}));
        }
        seen.push(Value::Array(mine));
    }
    seen
}

/// Whether `caller` signed in with a glave account.
fn with_account(caller: &Caller) -> bool {
    caller
        .auth
        .as_ref()
        .is_some_and(|auth| auth.method == AuthMethod::Account)
}
