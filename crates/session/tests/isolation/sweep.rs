//! The sweep: every person calls every method of the table with every other person's ids, and
//! each answer must be the one an id nobody has gets, with the ids put back as placeholders,
//! and must name nothing of anyone else's. What each person sees of their own is the same
//! before the sweep and after it.

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
        let org = &world.orgs[person.org];
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

/// Every id but those of `person`.
fn ids_of_others<L: EventLog + 'static>(world: &World<L>, person: usize) -> BTreeSet<String> {
    world
        .people
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != person)
        .flat_map(|(_, other)| other.ids.values().flatten())
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
    let mut found = Vec::new();
    for (a, attacker) in world.people.iter().enumerate() {
        let org = &world.orgs[attacker.org];
        let others = ids_of_others(world, a);
        for victim in world.people.iter().filter(|v| v.name != attacker.name) {
            for (method, templates) in table() {
                for template in &templates {
                    let codes = placeholders(template)
                        .iter()
                        .any(|(_, kind)| *kind == Kind::Code);
                    // A code is its holder's to sign in with; only another organization's is
                    // someone else's.
                    if codes && victim.org == attacker.org {
                        continue;
                    }
                    for variant in variants(template, attacker, victim) {
                        let (theirs, caller) = split(method, variant.params, &attacker.caller);
                        let (nobodys, _) = split(method, variant.nobodys, &attacker.caller);
                        let theirs = ask(org, &caller, theirs).await;
                        let nobodys = ask(org, &caller, nobodys).await;
                        let shown = normalized(&theirs, &variant.filled, false);
                        let expected = normalized(&nobodys, &variant.filled, true);
                        let named = others.iter().find(|id| shown.contains(id.as_str()));
                        if shown != expected || named.is_some() {
                            found.push(format!(
                                "{method} {template} by {} with {}'s ids {:?}:\n  \
                                 theirs: {shown}\n  nobody's: {expected}",
                                attacker.name,
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
        let org = &world.orgs[person.org];
        let mut mine = Vec::new();
        let mut call = async |method: &str, params: Value| {
            let answer = ask(org, &person.caller, request(method, params.clone())).await;
            mine.push(json!({"method": method, "params": params, "answer": answer}));
        };
        for thread in person.ids.get(&Kind::Thread).into_iter().flatten() {
            call("thread/get", json!({"thread": thread})).await;
            call("thread/events", json!({"thread": thread})).await;
            call("thread/sources", json!({"thread": thread})).await;
            call("goal/get", json!({"thread": thread})).await;
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
            "connection/list",
            "memory/list",
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
