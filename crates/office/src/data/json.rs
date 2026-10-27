//! JSON as a table: an array of objects, a row each, a column for each key in the order the
//! keys first appear.

use std::fmt;

use rusqlite::types::Value;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

use crate::error::OfficeError;

/// The rows of the JSON array of objects in `text`, from the file `name`, the first row its
/// keys.
pub(crate) fn rows(text: &str, name: &str) -> Result<Vec<Vec<Value>>, OfficeError> {
    let Objects(objects) = serde_json::from_str(text).map_err(|error| {
        OfficeError::Data(format!(
            "{name} is not a JSON array of objects, like [{{\"region\": \"North\", \"units\": \
             120}}], which is what glave doc sql loads: {error}"
        ))
    })?;
    let mut keys: Vec<String> = Vec::new();
    for object in &objects {
        for (key, _) in object {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    let mut rows = vec![keys.iter().cloned().map(Value::Text).collect()];
    for object in objects {
        let mut row = vec![Value::Null; keys.len()];
        for (key, json) in object {
            if let Some(cell) = keys
                .iter()
                .position(|known| *known == key)
                .and_then(|at| row.get_mut(at))
            {
                *cell = value(json);
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

/// A JSON value as SQLite holds it: true and false as 1 and 0, and an array or object as its
/// JSON text, for SQLite's JSON functions.
fn value(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(yes) => Value::Integer(i64::from(yes)),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(whole) => Value::Integer(whole),
            None => number.as_f64().map_or(Value::Null, Value::Real),
        },
        serde_json::Value::String(text) => Value::Text(text),
        json @ (serde_json::Value::Array(_) | serde_json::Value::Object(_)) => {
            Value::Text(json.to_string())
        }
    }
}

/// An object's keys and values, in its own order.
type Object = Vec<(String, serde_json::Value)>;

/// An array of objects, each keeping its keys' order.
struct Objects(Vec<Object>);

impl<'de> Deserialize<'de> for Objects {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_seq(Array)
    }
}

struct Array;

impl<'de> Visitor<'de> for Array {
    type Value = Objects;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an array of objects")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut array: A) -> Result<Objects, A::Error> {
        let mut objects = Vec::new();
        while let Some(InOrder(object)) = array.next_element()? {
            objects.push(object);
        }
        Ok(Objects(objects))
    }
}

/// One object, its keys in order.
struct InOrder(Object);

impl<'de> Deserialize<'de> for InOrder {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(Keys)
    }
}

struct Keys;

impl<'de> Visitor<'de> for Keys {
    type Value = InOrder;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<InOrder, A::Error> {
        let mut object = Vec::new();
        while let Some(entry) = map.next_entry()? {
            object.push(entry);
        }
        Ok(InOrder(object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_array_of_objects_is_a_row_each_with_its_keys_in_order() {
        let rows = rows(
            r#"[{"region": "North", "units": 120, "price": 19.5, "paid": true},
                {"units": 80, "region": "South", "tags": ["a"], "price": null}]"#,
            "a.json",
        )
        .unwrap();
        let text = |text: &str| Value::Text(text.to_owned());
        assert_eq!(
            rows,
            [
                vec![
                    text("region"),
                    text("units"),
                    text("price"),
                    text("paid"),
                    text("tags")
                ],
                vec![
                    text("North"),
                    Value::Integer(120),
                    Value::Real(19.5),
                    Value::Integer(1),
                    Value::Null
                ],
                vec![
                    text("South"),
                    Value::Integer(80),
                    Value::Null,
                    Value::Null,
                    text("[\"a\"]")
                ],
            ]
        );
    }

    #[test]
    fn json_that_is_not_an_array_of_objects_says_what_is_loaded() {
        for wrong in [r#"{"a": 1}"#, "[1, 2]", "[{"] {
            let error = rows(wrong, "a.json").unwrap_err().to_string();
            assert!(
                error.starts_with("a.json is not a JSON array of objects, like [{"),
                "{error}"
            );
        }
    }
}
