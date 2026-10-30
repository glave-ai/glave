//! The instructions ledger's record (`specs/computers.md`, decision 6). `instructions.read`: what
//! was last read on a person's own computer for a project's folder, file by file, so a thread
//! there opens with it while that computer sleeps; the latest one holds.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};

pub(crate) const READ: &str = "instructions.read";

/// The instructions read on a person's own computer for a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionsRead {
    /// The folder they were read for, as that computer names it.
    pub root: String,
    /// Each file read, in the order a thread is sent them; none when the folder has none.
    pub files: Vec<InstructionsFile>,
}

/// One file of instructions, as a thread is sent it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionsFile {
    /// Where it is, as that computer names it.
    pub path: String,
    /// Its text as sent: trimmed, and cut short within the limit on instructions.
    pub text: String,
}

impl Payload for InstructionsRead {
    const TYPE: &'static str = READ;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_read(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<InstructionsRead>> {
    (version == 1).then(|| parse(data))
}
