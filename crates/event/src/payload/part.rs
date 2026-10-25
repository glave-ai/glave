//! Content parts, shared by messages and tool results.

use serde::{Deserialize, Serialize};

use crate::blob::BlobRef;

/// One piece of a message or a tool result, kept in its original order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Part {
    Text {
        text: String,
    },
    /// Reasoning returned by the model: readable text, an opaque provider token, or both.
    Reasoning {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opaque: Option<Opaque>,
    },
    /// Content held in the blob store, such as an image, a document or a large output.
    File {
        blob: BlobRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// The copy of an image the model is sent in its place, made once as the image came
        /// into the thread: smaller and upright. None when the image is sent as it is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resized: Option<BlobRef>,
    },
}

impl Part {
    /// The blob the model is sent for a file: its resized copy when it has one.
    pub fn sent(&self) -> Option<&BlobRef> {
        match self {
            Self::File { blob, resized, .. } => Some(resized.as_ref().unwrap_or(blob)),
            Self::Text { .. } | Self::Reasoning { .. } => None,
        }
    }
}

/// Provider data that must be sent back byte for byte, such as a reasoning signature or
/// encrypted reasoning. It is replayed only to the provider and model that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opaque {
    pub provider: String,
    pub model: String,
    pub kind: String,
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_are_tagged_by_type() {
        let part = Part::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert_eq!(json, r#"{"type":"text","text":"hello"}"#);
        assert_eq!(serde_json::from_str::<Part>(&json).unwrap(), part);
    }

    #[test]
    fn unknown_part_fields_and_kinds_are_rejected() {
        assert!(serde_json::from_str::<Part>(r#"{"type":"text","text":"hi","extra":1}"#).is_err());
        assert!(serde_json::from_str::<Part>(r#"{"type":"video","url":"x"}"#).is_err());
    }
}
