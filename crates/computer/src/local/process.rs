//! A command's output on this machine once it runs: read as text, a chunk at a time, without
//! losing a chunk when reading stops part way. Its process group is stopped whole through
//! glave-process's `Child`.

use glave_ports::computer::Output;
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, ChildStdout};

/// How much is read from a command's output at once.
const CHUNK: usize = 16 * 1024;

/// A command's output as it is read: standard output and error together, decoded as text,
/// with what was read but not yet handed on.
#[derive(Debug)]
pub(crate) struct Stream {
    stdout: Option<ChildStdout>,
    buffer: Vec<u8>,
    text: Utf8,
    /// Decoded, and not yet taken by whoever reads the stream.
    unsent: String,
}

impl Stream {
    pub(crate) fn of(child: &mut Child) -> Self {
        Self {
            stdout: child.stdout.take(),
            buffer: vec![0u8; CHUNK],
            text: Utf8::default(),
            unsent: String::new(),
        }
    }

    /// Reads the next chunk into what is unsent; false once the output has closed. Safe to
    /// stop at any point: nothing read is lost.
    pub(crate) async fn read(&mut self) -> bool {
        let Some(stdout) = self.stdout.as_mut() else {
            return false;
        };
        match stdout.read(&mut self.buffer).await {
            Ok(read) if read > 0 => {
                let decoded = self.text.push(self.buffer.get(..read).unwrap_or_default());
                self.unsent.push_str(&decoded);
                true
            }
            _ => {
                self.stdout = None;
                let rest = self.text.finish();
                self.unsent.push_str(&rest);
                false
            }
        }
    }

    /// What was read and not yet taken.
    pub(crate) fn take(&mut self) -> String {
        std::mem::take(&mut self.unsent)
    }

    /// Hands the output to `output` as it arrives, until it closes. Stopped part way, it keeps
    /// what `output` did not take yet.
    pub(crate) async fn pump(&mut self, output: &dyn Output) {
        loop {
            if !self.unsent.is_empty() {
                output.write(&self.unsent).await;
                self.unsent.clear();
            }
            if !self.read().await && self.unsent.is_empty() {
                return;
            }
        }
    }
}

/// Turns bytes into text without splitting a character across chunks.
#[derive(Debug, Default)]
pub(crate) struct Utf8 {
    pending: Vec<u8>,
}

impl Utf8 {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let cut = match std::str::from_utf8(&self.pending) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => self.pending.len(),
        };
        let rest = self.pending.split_off(cut);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }

    pub(crate) fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}
