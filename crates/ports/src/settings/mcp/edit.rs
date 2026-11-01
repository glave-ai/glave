//! A `.mcp.json` file's text changed by glave, as the person or a project wrote it: its members
//! in their order, each value's text as it was, so only the server glave adds or takes out
//! changes; only the spacing between them is glave's.

use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;

use super::McpSettingsError;

/// The member that holds the servers.
const SERVERS: &str = "mcpServers";

/// A JSON object's members in the order they were written, each value's text as it was.
#[derive(Debug, Default)]
pub struct Members(Vec<(String, Box<RawValue>)>);

impl Members {
    /// The member `name`'s text.
    pub fn get(&self, name: &str) -> Option<&RawValue> {
        let found = self.0.iter().find(|(each, _)| each == name);
        found.map(|(_, raw)| raw.as_ref())
    }

    /// Sets `name` to `value`: in its place when there is one, else last.
    pub fn set(&mut self, name: &str, value: Box<RawValue>) {
        match self.0.iter_mut().find(|(each, _)| each == name) {
            Some((_, raw)) => *raw = value,
            None => self.0.push((name.to_owned(), value)),
        }
    }

    /// Keeps only the members `keep` keeps, by name.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.0.retain(|(name, _)| keep(name));
    }
}

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct InOrder;
        impl<'de> Visitor<'de> for InOrder {
            type Value = Members;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Members, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry::<String, Box<RawValue>>()? {
                    members.push(member);
                }
                Ok(Members(members))
            }
        }
        deserializer.deserialize_map(InOrder)
    }
}

impl Serialize for Members {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in &self.0 {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

/// A `.mcp.json` file read for a change: its servers to change, the rest kept as it was.
#[derive(Debug, Default)]
pub struct Edited {
    file: Members,
    /// The servers, by name, in their order.
    pub servers: Members,
}

impl Edited {
    /// The file whose text is `text`; empty text is a file with no servers.
    pub fn read(text: &str) -> Result<Self, McpSettingsError> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let unread = |error: serde_json::Error| McpSettingsError(error.to_string());
        let file: Members = serde_json::from_str(text).map_err(unread)?;
        let servers = match file.get(SERVERS) {
            Some(raw) => serde_json::from_str(raw.get()).map_err(unread)?,
            None => Members::default(),
        };
        Ok(Self { file, servers })
    }

    /// The file's text with its servers as they are now, ending in a new line.
    pub fn text(mut self) -> Result<String, McpSettingsError> {
        let failed = |error: serde_json::Error| McpSettingsError(error.to_string());
        let servers = serde_json::to_string_pretty(&self.servers).map_err(failed)?;
        let servers = RawValue::from_string(servers).map_err(failed)?;
        self.file.set(SERVERS, servers);
        let mut text = serde_json::to_string_pretty(&self.file).map_err(failed)?;
        text.push('\n');
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(text: &str) -> Box<RawValue> {
        RawValue::from_string(text.to_owned()).unwrap()
    }

    #[test]
    fn a_server_is_added_and_taken_out_with_the_rest_as_written() {
        let text = r#"{"note": "mine", "mcpServers": {"b": {"command": "b",  "args": ["x"]}}}"#;
        let mut edited = Edited::read(text).unwrap();
        edited
            .servers
            .set("a", raw(r#"{"type":"http","url":"https://a.test/mcp"}"#));
        let added = edited.text().unwrap();
        assert!(added.starts_with("{\n  \"note\": \"mine\""), "{added}");
        assert!(
            added.contains(r#""b": {"command": "b",  "args": ["x"]}"#),
            "{added}"
        );
        assert!(
            added.find("\"b\"") < added.find("\"a\""),
            "added last: {added}"
        );
        let mut edited = Edited::read(&added).unwrap();
        edited.servers.retain(|name| name != "a");
        let back = edited.text().unwrap();
        assert!(!back.contains("a.test"), "{back}");
        assert!(back.contains("\"note\""), "{back}");
    }

    #[test]
    fn an_empty_file_gets_its_servers_and_a_broken_one_is_refused() {
        let mut edited = Edited::read("").unwrap();
        assert!(edited.servers.get("a").is_none());
        edited.servers.set("a", raw(r#"{"command":"a"}"#));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&edited.text().unwrap()).unwrap(),
            serde_json::json!({"mcpServers": {"a": {"command": "a"}}})
        );
        assert!(Edited::read("{not json").is_err());
    }
}
