//! The one gate every tool result passes before it is recorded or sent (`specs/invariants.md`,
//! 22): the history holds words, never files. Bytes a call handed over, base64 found in its
//! text and text too long for one piece are kept as sealed blobs, and the history and the model
//! get a short line naming each ([`scan`]).

mod scan;

use glave_event::{Part, Sha256, ThreadId};
use glave_ports::{BlobStore, EventLog, ModelProvider, Pieces, Scope, ToolOutput, digest};
use serde_json::Value;

use crate::approval::Approver;
use crate::harness::Harness;
use crate::untrusted::{defuse, label_bytes};

use scan::{MOST_TEXT, sweep};

/// The most one thing a call gives may be, in bytes; more is not kept, and the line says so.
const MOST_KEPT: u64 = 256 * 1024 * 1024;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// `output` as the history may hold it, and the model read it: what the gate pulled out is
    /// kept for `thread`, and what could not be kept is said so. Nothing it pulled out stays in
    /// the text, kept or not.
    pub(crate) async fn gated(
        &self,
        scope: &Scope,
        thread: ThreadId,
        output: ToolOutput,
    ) -> ToolOutput {
        // Reading and hashing what may be megabytes, off the async threads.
        let passed = tokio::task::spawn_blocking(move || pass(output)).await;
        let Ok((mut output, saved)) = passed else {
            return ToolOutput::text(true, "[this result could not be read]");
        };
        for (reference, media_type, bytes) in saved {
            let kept = self.blobs.put(
                scope,
                Some(thread),
                media_type,
                MOST_KEPT,
                Box::new(Pieces::from(bytes)),
            );
            if let Err(error) = kept.await {
                output.parts.push(Part::Text {
                    text: format!("[blob:{} could not be kept: {error}]", reference.as_str()),
                });
            }
        }
        output
    }
}

/// One thing a result held beyond the limits: its name, what it is, and its bytes.
type Kept = (Sha256, &'static str, Vec<u8>);

/// `output` within the limits, with what it held beyond them.
fn pass(mut output: ToolOutput) -> (ToolOutput, Vec<Kept>) {
    let mut saved = std::mem::take(&mut output.saved);
    // Untrusted text is labelled after this, its closing tags broken up, within the same cap.
    let untrusted = output.untrusted;
    let room = MOST_TEXT.saturating_sub(if untrusted { label_bytes() } else { 0 });
    for part in &mut output.parts {
        if let Part::Text { text } = part {
            if untrusted {
                *text = defuse(text, "untrusted");
            }
            sweep(text, room, &mut saved);
        }
    }
    if let Some(structured) = &mut output.structured {
        sweep_strings(structured, room, &mut saved);
    }
    let saved = saved
        .into_iter()
        .map(|bytes| (digest(&bytes), scan::media_type(&bytes), bytes))
        .collect();
    (output, saved)
}

/// Every string in `value` swept where it is, however a call labelled it.
fn sweep_strings(value: &mut Value, room: usize, saved: &mut Vec<Vec<u8>>) {
    match value {
        Value::String(text) => sweep(text, room, saved),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| sweep_strings(item, room, saved)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| sweep_strings(field, room, saved)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
