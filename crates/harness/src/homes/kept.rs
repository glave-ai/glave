//! What was last read on a person's own computer for a project's folder, or for their chats
//! their own instructions, kept in the project's instructions ledger (`instructions.read`), so a
//! thread there opens with it without reaching that computer (`specs/computers.md`, decision 6).
//! The latest one holds; a read the same as it records nothing.

use std::path::Path;

use glave_event::{Body, InstructionsRead, Payload as _, Seq, decode};
use glave_ports::stream::{Reserved, append_retrying};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope};

use crate::approval::Approver;
use super::ReadThere;
use crate::harness::{Harness, HarnessError};

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// What was last read for `root` in `scope`'s project, when that read was for `root`: the
    /// project's folder then is the folder now.
    pub(crate) async fn last_read(&self, scope: Scope, root: &Path) -> Option<InstructionsRead> {
        match self.latest_read(scope).await {
            Ok(Some((_, Some(read)))) if Path::new(&read.root) == root => Some(read),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, "could not read what a project's folder opens with");
                None
            }
        }
    }

    /// Keeps what was just read for `root` as what `scope`'s project's threads there open with (for a chat, the person's own, read for their home), unless they are what
    /// was kept already.
    pub(crate) async fn keep_read(
        &self,
        scope: Scope,
        root: &Path,
        ReadThere {
            files,
            skills,
            config,
        }: ReadThere,
    ) {
        let read = InstructionsRead {
            root: root.display().to_string(),
            files,
            skills,
            config,
        };
        let ledger = Reserved::Instructions.id();
        let (this, read) = (self, &read);
        let kept = append_retrying(&self.log, &scope, ledger, move || async move {
            let latest = this.latest_read(scope).await?;
            if latest
                .as_ref()
                .is_some_and(|(_, kept)| kept.as_ref() == Some(read))
            {
                return Ok(None);
            }
            let seq = latest
                .map_or(Some(Seq::FIRST), |(seq, _)| seq.next())
                .ok_or_else(|| HarnessError::Event("the instructions ledger is full".into()))?;
            let mut events = Vec::new();
            this.stage_next(ledger, seq, &mut events, read)?;
            Ok::<_, HarnessError>(Some((events, ())))
        })
        .await;
        if let Err(error) = kept {
            tracing::warn!(%error, "could not keep what a project's folder opens with");
        }
    }

    /// The `seq` of the latest read kept in `scope`'s project, the ledger's last, and the read
    /// when this glave knows its version.
    async fn latest_read(
        &self,
        scope: Scope,
    ) -> Result<Option<(Seq, Option<InstructionsRead>)>, HarnessError> {
        let ledger = Reserved::Instructions.id();
        let latest = self
            .log
            .latest_of(&scope, ledger, &[InstructionsRead::TYPE])
            .await?;
        let Some(raw) = latest else {
            return Ok(None);
        };
        let seq = raw.seq;
        let event = decode(raw).map_err(|error| HarnessError::Event(error.to_string()))?;
        Ok(Some(match event.body {
            Body::InstructionsRead(read) => (seq, Some(read)),
            _ => (seq, None),
        }))
    }
}
