//! Files a turn made for the person (`specs/session.md`, "Files it made"): how many pages,
//! sheets or slides one has, as glave read it.

use serde::{Deserialize, Serialize};

/// How many of its parts a file has: a document's pages, a workbook's sheets, a deck's slides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Parts {
    Pages(u64),
    Sheets(u64),
    Slides(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_are_written_by_what_they_count() {
        let written = serde_json::to_string(&Parts::Sheets(2)).unwrap();
        assert_eq!(written, r#"{"sheets":2}"#);
        assert_eq!(
            serde_json::from_str::<Parts>(&written).unwrap(),
            Parts::Sheets(2)
        );
    }
}
