//! The session API as one JSON Schema document (`specs/session.schema.json`), generated from
//! the protocol types themselves: every method with its params and result, every notification,
//! and every error code. `cargo xtask schema` writes it and the TypeScript client made from it;
//! CI fails when the checked-in copy and the code disagree.

use schemars::generate::SchemaSettings;
use serde_json::{Map, Value, json};

use crate::protocol::{ApiError, Notification, PROTOCOL, RESULTS, Reply, Request, code};

/// The whole document, or what in the protocol types it could not describe.
pub fn document() -> Result<Value, String> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let request = generator.root_schema_for::<Request>().to_value();
    let reply = generator.root_schema_for::<Reply>().to_value();
    let notification = generator.root_schema_for::<Notification>().to_value();
    let failure = generator.subschema_for::<ApiError>().to_value();
    let mut defs = Map::new();
    for root in [&request, &reply, &notification] {
        if let Some(Value::Object(found)) = root.get("$defs") {
            defs.extend(found.clone());
        }
    }
    defs.extend(
        generator
            .take_definitions(true)
            .into_iter()
            .collect::<Map<_, _>>(),
    );
    let replies = by_title(&reply)?;
    let mut methods = Map::new();
    for (method, params) in variants(&request)? {
        let titles = RESULTS
            .iter()
            .find_map(|(name, titles)| (*name == method).then_some(*titles))
            .ok_or_else(|| format!("{method} has no result"))?;
        let mut results = Vec::new();
        for title in titles {
            let reply = replies
                .get(*title)
                .ok_or_else(|| format!("no reply is titled {title}"))?;
            results.push(reply.clone());
        }
        let result = match results.as_slice() {
            [one] => one.clone(),
            _ => json!({ "anyOf": results }),
        };
        methods.insert(
            method.clone(),
            json!({
                "params": params,
                "result": result,
                // A method that can answer with a turn streams it, unless run as a task.
                "streams": titles.contains(&"Turn"),
            }),
        );
    }
    let notifications: Map<String, Value> = variants(&notification)?.into_iter().collect();
    let errors: Map<String, Value> = code::ALL
        .iter()
        .map(|(code, name, meaning)| {
            (
                (*name).to_owned(),
                json!({"code": code, "message": meaning}),
            )
        })
        .collect();
    Ok(sorted(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "glave session API",
        "description": "JSON-RPC 2.0 methods, notifications and errors of glave's session API \
                        (specs/session.md). Generated from the Rust protocol types by \
                        `cargo xtask schema`; do not edit.",
        "protocol": PROTOCOL,
        "methods": methods,
        "notifications": notifications,
        "error": failure,
        "errors": errors,
        "$defs": defs,
    })))
}

/// `value` with every object's members in name order, so the document reads the same however
/// the JSON library keeps them.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut fields: Vec<(String, Value)> = fields.into_iter().collect();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                fields
                    .into_iter()
                    .map(|(name, value)| (name, sorted(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// The variants of an adjacently tagged enum's schema: each `method` and its `params`.
fn variants(root: &Value) -> Result<Vec<(String, Value)>, String> {
    let cases = root
        .get("oneOf")
        .and_then(Value::as_array)
        .ok_or("a tagged enum's schema has no oneOf")?;
    cases
        .iter()
        .map(|case| {
            let properties = case
                .get("properties")
                .ok_or("a variant has no properties")?;
            let method = properties
                .pointer("/method/const")
                .and_then(Value::as_str)
                .ok_or("a variant has no method")?;
            let params = properties
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"}));
            Ok((method.to_owned(), params))
        })
        .collect()
}

/// The cases of the untagged [`Reply`] by their titles, the titles taken off.
fn by_title(root: &Value) -> Result<Map<String, Value>, String> {
    let cases = root
        .get("anyOf")
        .and_then(Value::as_array)
        .ok_or("the reply's schema has no anyOf")?;
    cases
        .iter()
        .map(|case| {
            let mut case = case.clone();
            let title = case
                .as_object_mut()
                .and_then(|fields| fields.remove("title"))
                .and_then(|title| title.as_str().map(str::to_owned))
                .ok_or("a reply has no title")?;
            Ok((title, case))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_is_listed_once() {
        let mut codes: Vec<i64> = code::ALL.iter().map(|(code, ..)| *code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), code::ALL.len());
    }
}
