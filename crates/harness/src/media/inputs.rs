//! The images a media call draws on, as the model named them: a file the thread holds
//! (`blob:<sha256>`) or a file on its computer by its path, each read, made fit to send and
//! handed to the provider inline, in order.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glave_event::{BlobRef, Part, Sha256};
use glave_ports::computer::EntryKind;
use glave_ports::secrets::is_secret_file;
use glave_ports::{BlobStore, EventLog, InlineFile, ModelProvider, digest, is_image, read_all};

use crate::approval::Approver;
use crate::files::{MAX_FILE, sniffed};
use crate::harness::{Harness, Thread};

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
            let (bytes, copy) = self.fit(bytes).await;
            let (media_type, bytes) = match copy {
                Ok(Some(copy)) => ("image/jpeg", copy),
                Ok(None) => (media_type, bytes),
                Err(error) => return Err(format!("{name} can't be sent: {error}.")),
            };
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

    /// The bytes of the file `hex` names, `name` as the model gave it, which `thread`'s
    /// project holds.
    async fn held(&self, thread: &Thread, name: &str, hex: &str) -> Result<Vec<u8>, String> {
        let gone = || format!("{name} is not a file this thread holds.");
        let sha256 = Sha256::new(hex.trim()).map_err(|_| gone())?;
        let found = self.blobs.get(&thread.scope, &sha256).await;
        let mut found = found.map_err(|error| format!("{name} can't be read: {error}."))?;
        let found = found.as_mut().ok_or_else(gone)?;
        read_all(found.bytes.as_mut(), MAX_FILE)
            .await
            .map_err(|error| format!("{name} can't be read: {error}."))
    }

    /// The bytes of the file at `path` on `thread`'s computer, never one that usually holds
    /// secrets.
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
        if stat.size > MAX_FILE {
            return Err(format!("{shown} is too large: at most {MAX_FILE} bytes."));
        }
        let read = workspace.read(&shown).await;
        read.map(glave_ports::computer::Bytes::into_vec)
            .map_err(|error| format!("Could not read {shown}: {}", error.message()))
    }
}
