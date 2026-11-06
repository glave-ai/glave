//! Whether a call's arguments fit its tool's parameters, as far as the top of its JSON Schema
//! says: checked before a connected tool runs, since the model may have called it without
//! seeing them.

use serde_json::{Map, Value};

/// What is wrong with `arguments` by `schema`, when anything is: a required one missing, one it
/// does not name where it takes no others, or one of the wrong type or not among its values.
pub(super) fn problem(schema: &Value, arguments: &Map<String, Value>) -> Option<String> {
    let required = schema.get("required").and_then(Value::as_array);
    for name in required.into_iter().flatten().filter_map(Value::as_str) {
        if !arguments.contains_key(name) {
            return Some(format!("the required argument {name:?} is missing"));
        }
    }
    let properties = schema.get("properties").and_then(Value::as_object)?;
    let open = !matches!(
        schema.get("additionalProperties"),
        None | Some(Value::Bool(false))
    );
    for (name, value) in arguments {
        let Some(property) = properties.get(name) else {
            if open {
                continue;
            }
            let known: Vec<&String> = properties.keys().collect();
            return Some(format!("it takes no argument {name:?}, only {known:?}"));
        };
        if value.is_null() || fits(property, value) {
            continue;
        }
        return Some(format!("the argument {name:?} is not what it takes"));
    }
    None
}

/// Whether `value` is of a type `property` allows, and among its values when it lists them.
fn fits(property: &Value, value: &Value) -> bool {
    let of_type = |kind: &str| match kind {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    };
    let typed = match property.get("type") {
        Some(Value::String(kind)) => of_type(kind),
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).any(of_type),
        _ => true,
    };
    let listed = property
        .get("enum")
        .and_then(Value::as_array)
        .is_none_or(|values| values.contains(value));
    typed && listed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn what_fits_runs_and_what_does_not_says_why() {
        let schema = json!({"type": "object", "properties": {
            "number": {"type": "integer"},
            "method": {"type": "string", "enum": ["get", "get_comments"]},
            "labels": {"type": ["array", "null"]}
        }, "required": ["number"]});
        let fine = args(&json!({"number": 4120, "method": "get", "labels": null}));
        assert_eq!(problem(&schema, &fine), None);
        let missing = problem(&schema, &args(&json!({"method": "get"}))).unwrap();
        assert!(missing.contains("\"number\" is missing"), "{missing}");
        let typed = problem(&schema, &args(&json!({"number": "4120"}))).unwrap();
        assert!(typed.contains("\"number\""), "{typed}");
        let listed = problem(&schema, &args(&json!({"number": 1, "method": "list"}))).unwrap();
        assert!(listed.contains("\"method\""), "{listed}");
        let unknown = problem(&schema, &args(&json!({"number": 1, "issue": 1}))).unwrap();
        assert!(
            unknown.contains("\"issue\"") && unknown.contains("number"),
            "{unknown}"
        );
    }

    #[test]
    fn a_schema_open_to_more_takes_more() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}},
            "additionalProperties": true});
        assert_eq!(problem(&schema, &args(&json!({"a": "x", "b": 1}))), None);
        assert_eq!(
            problem(&json!({"type": "object"}), &args(&json!({"b": 1}))),
            None
        );
    }
}
