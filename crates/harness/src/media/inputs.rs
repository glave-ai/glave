//! The images a media call draws on, as the model named them: a file the thread holds
//! (`blob:<sha256>`, one a `file` part of its own messages or tool results names, never another
//! thread's) or a file on its computer by its path, each read and checked before anyone is
//! asked, and handed to the provider whole, as it is, in order: only the model's own copy of an
//! image is made smaller.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{BlobRef, Body, Part, Sha256, decode};
use glave_ports::computer::EntryKind;
use glave_ports::secrets::is_secret_file;
use glave_ports::{
    BlobError, BlobStore, EventLog, InlineFile, Message, ModelProvider, digest, is_image, read_all,
};

use crate::approval::Approver;
use crate::files::sniffed;
use crate::harness::{Harness, Thread};

/// The largest image sent to an image model, in bytes: what they take in.
const MOST_SENT: u64 = 20 * 1024 * 1024;

/// How many of a thread's events are read at once, looking for a file it holds.
const PAGE: usize = 500;

/// The images a call draws on: a `file` part for each, in order, and its content inline.
pub(super) struct Inputs {
    pub(super) parts: Vec<Part>,
    pub(super) files: Vec<InlineFile>,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The images `named`, in order, as `thread` holds them or its computer does, or what the
    /// model is told is wrong with one.
    pub(super) async fn inputs(
        &self,
        thread: &Thread,
        named: impl Iterator<Item = &String>,
    ) -> Result<Inputs, String> {
        let mut inputs = Inputs {
            parts: Vec::new(),
            files: Vec::new(),
        };
        for name in named {
            let name = name.trim();
            let bytes = match name.strip_prefix("blob:") {
                Some(hex) => self.held(thread, name, hex).await?,
                None => self.on_computer(thread, name).await?,
            };
            let media_type = sniffed(&bytes)
                .filter(|kind| is_image(kind))
                .ok_or_else(|| {
                    format!("{name} is not an image glave reads: a PNG, JPEG, GIF or WebP.")
                })?;
            let sha256 = digest(&bytes);
            inputs.parts.push(Part::File {
                blob: BlobRef {
                    sha256: sha256.clone(),
                    bytes: bytes.len() as u64,
                    media_type: media_type.to_owned(),
                },
                name: None,
                resized: None,
            });
            inputs.files.push(InlineFile {
                sha256,
                media_type: media_type.to_owned(),
                data: STANDARD.encode(&bytes),
            });
        }
        Ok(inputs)
    }

    /// The bytes of the file `hex` names, `name` as the model gave it, which `thread` holds.
    async fn held(&self, thread: &Thread, name: &str, hex: &str) -> Result<Vec<u8>, String> {
        let gone = || format!("{name} is not a file this thread holds.");
        let sha256 = Sha256::new(hex.trim()).map_err(|_| gone())?;
        if !self.holds(thread, &sha256).await? {
            return Err(gone());
        }
        let found = self.blobs.get(&thread.scope, &sha256).await;
        let mut found = found.map_err(|error| format!("{name} can't be read: {error}."))?;
        let found = found.as_mut().ok_or_else(gone)?;
        read_all(found.bytes.as_mut(), MOST_SENT)
            .await
            .map_err(|error| match error {
                BlobError::TooLarge(_) => too_large(name),
                BlobError::Storage(_) => format!("{name} can't be read: {error}."),
            })
    }

    /// Whether a `file` part of `thread`'s own messages or tool results names `sha256`, as its
    /// conversation holds them now, else as its log does, a compaction having left them behind.
    async fn holds(&self, thread: &Thread, sha256: &Sha256) -> Result<bool, String> {
        let now = thread.messages().iter().any(|message| match message {
            Message::User { parts } | Message::Assistant { parts, .. } => names(parts, sha256),
            Message::Tool { output, .. } => names(output, sha256),
        });
        if now {
            return Ok(true);
        }
        let mut after = None;
        loop {
            let page = self.log.read(&thread.scope, thread.id, after, PAGE).await;
            let page =
                page.map_err(|error| format!("this thread's files can't be read: {error}"))?;
            let named = page
                .iter()
                .any(|raw| match decode(raw.clone()).map(|event| event.body) {
                    Ok(Body::MessageAdded(added)) => names(&added.parts, sha256),
                    Ok(Body::ToolCallCompleted(completed)) => names(&completed.output, sha256),
                    _ => false,
                });
            if named {
                return Ok(true);
            }
            if page.len() < PAGE {
                return Ok(false);
            }
            after = page.last().map(|raw| raw.seq);
        }
    }

    /// The bytes of the file at `path` on `thread`'s computer, confined to its workspace, never
    /// one that usually holds secrets.
    async fn on_computer(&self, thread: &Thread, path: &str) -> Result<Vec<u8>, String> {
        let workspace = self.workspace_for(thread);
        let stat = workspace.stat(path).await?;
        let shown = stat.path;
        if is_secret_file(Path::new(&shown)) {
            return Err(format!("{shown} usually holds secrets: it is never sent."));
        }
        match stat.kind {
            Some(EntryKind::File | EntryKind::Link) => {}
            Some(EntryKind::Folder) => return Err(format!("{shown} is a folder.")),
            None => return Err(format!("File not found: {shown}")),
        }
        if stat.size > MOST_SENT {
            return Err(too_large(&shown));
        }
        let read = workspace.read(&shown).await;
        read.map(glave_ports::computer::Bytes::into_vec)
            .map_err(|error| format!("Could not read {shown}: {}", error.message()))
    }
}

/// Whether a `file` part of `parts` names `sha256`, as the file or the copy made fit to send.
fn names(parts: &[Part], sha256: &Sha256) -> bool {
    parts.iter().any(|part| match part {
        Part::File { blob, resized, .. } => {
            blob.sha256 == *sha256 || resized.as_ref().is_some_and(|copy| copy.sha256 == *sha256)
        }
        Part::Text { .. } | Part::Reasoning { .. } => false,
    })
}

/// What the model is told of an image too large to send, `name` as it named it.
fn too_large(name: &str) -> String {
    let most = MOST_SENT / (1024 * 1024);
    format!("{name} is larger than the {most} MB an image model takes: send a smaller copy.")
}
