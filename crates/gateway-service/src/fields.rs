//! Reading a request's JSON fields, for both surfaces: each field taken as it is read, so what
//! is left at the end is what the gateway does not understand, and is refused by name.

use serde_json::{Map, Value};

use crate::errors::Refusal;

/// Refuses whatever is left of `fields` but those `ignored`, naming the first.
pub(crate) fn refuse_the_rest(
    fields: &Map<String, Value>,
    ignored: &[&str],
) -> Result<(), Refusal> {
    match fields
        .keys()
        .find(|field| !ignored.contains(&field.as_str()))
    {
        Some(field) => Err(Refusal::unsupported(
            field,
            "the gateway passes on only what every model it routes to can honour",
        )),
        None => Ok(()),
    }
}

pub(crate) fn take_string(
    fields: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<String>, Refusal> {
    match fields.remove(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(Refusal::invalid(format!("{name} must be a string"))),
    }
}

pub(crate) fn take_bool(
    fields: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<bool>, Refusal> {
    match fields.remove(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(value)),
        Some(_) => Err(Refusal::invalid(format!("{name} must be true or false"))),
    }
}

pub(crate) fn take_count(
    fields: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<u32>, Refusal> {
    match fields.remove(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|count| u32::try_from(count).ok())
            .filter(|count| *count >= 1)
            .map(Some)
            .ok_or_else(|| Refusal::invalid(format!("{name} must be a whole number above 0"))),
    }
}

/// The text of `content`: a string, or a list of text parts (`{"type": "text", "text": …}`,
/// as both surfaces write them), joined by lines. Any other part is refused.
pub(crate) fn text_of(content: &Value, at: &str) -> Result<String, Refusal> {
    match content {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Array(parts) => {
            let mut texts = Vec::with_capacity(parts.len());
            for part in parts {
                match (part.get("type").and_then(Value::as_str), part.get("text")) {
                    (Some("text"), Some(Value::String(text))) => texts.push(text.as_str()),
                    (kind, _) => {
                        return Err(Refusal::unsupported(
                            &format!("{at} parts of type {}", kind.unwrap_or("(none)")),
                            "only text is passed on yet",
                        ));
                    }
                }
            }
            Ok(texts.join("\n"))
        }
        _ => Err(Refusal::invalid(format!(
            "{at} must be text or a list of parts"
        ))),
    }
}
