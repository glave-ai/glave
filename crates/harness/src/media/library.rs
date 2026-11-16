//! What a media tool made, kept in the person's Library (`specs/session.md`, "Library"): an
//! item in the project's Library ledger that holds the image under a holder of its own, beside
//! the thread's, so it outlives the chat. Held first, then written down, so a failure leaves a
//! hold too many rather than an item whose file may go.

use glave_event::{AssetId, AssetKind, BlobRef, LibraryKept, LibraryRemoved, Payload as _, Seq};
use glave_ports::stream::{Reserved, append_retrying};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};

/// The longest name an item keeps, in characters, before it is cut at a word.
const LONGEST_NAME: usize = 60;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Keeps `blob`, which `thread` holds, in its project's Library as a picture made from
    /// `prompt`, saved as `file_name`: the item's id.
    pub(crate) async fn keep_in_library(
        &self,
        thread: &Thread,
        blob: &BlobRef,
        prompt: &str,
        file_name: String,
    ) -> Result<AssetId, HarnessError> {
        let now = self.clock.now();
        let asset = AssetId::new(now, self.entropy.bytes()?)?;
        let holder = glave_event::ThreadId::new(now, self.entropy.bytes()?)?;
        if !self
            .blobs
            .refer(&thread.scope, &blob.sha256, holder)
            .await?
        {
            return Err(HarnessError::Event("the image is no longer kept".into()));
        }
        let kept = LibraryKept {
            asset,
            kind: AssetKind::Picture,
            name: name(prompt),
            file_name,
            blob: blob.clone(),
            holder,
            thread: thread.id,
        };
        let (scope, ledger, kept) = (&thread.scope, Reserved::Library.id(), &kept);
        append_retrying(&self.log, scope, ledger, move || async move {
            let kinds = [LibraryKept::TYPE, LibraryRemoved::TYPE];
            let last = self.log.latest_of(scope, ledger, &kinds).await?;
            let seq = last
                .map_or(Some(Seq::FIRST), |raw| raw.seq.next())
                .ok_or_else(|| HarnessError::Event("the Library ledger is full".into()))?;
            let mut events = Vec::new();
            self.stage_next(ledger, seq, &mut events, kept)?;
            Ok::<_, HarnessError>(Some((events, ())))
        })
        .await?;
        Ok(asset)
    }
}

/// What an item made from `prompt` is called: its words, up to [`LONGEST_NAME`] characters cut
/// at a word, the first letter capital; `Picture` when it has none.
fn name(prompt: &str) -> String {
    let mut name = String::new();
    for word in prompt.split_whitespace() {
        let longer = name.chars().count() + word.chars().count() + 1;
        if !name.is_empty() && longer > LONGEST_NAME {
            name.push('…');
            break;
        }
        if !name.is_empty() {
            name.push(' ');
        }
        name.extend(word.chars().take(LONGEST_NAME));
    }
    let mut letters = name.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => "Picture".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::name;

    #[test]
    fn a_name_is_the_prompt_cut_at_a_word_with_a_capital() {
        assert_eq!(name("a fox at dawn"), "A fox at dawn");
        assert_eq!(name("  "), "Picture");
        let long =
            "a watercolour of a red fox sitting in tall grass at dawn with mist over the hills";
        let named = name(long);
        assert!(
            named.ends_with('…') && named.chars().count() <= 61,
            "{named}"
        );
        assert!(named.starts_with("A watercolour of a red fox"), "{named}");
    }
}
