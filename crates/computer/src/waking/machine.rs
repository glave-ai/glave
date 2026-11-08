//! What a computer that lost its machine says (`specs/computers.md`, decision 16): the call it
//! cut off says why it stopped, and its commands in the background are gone with it, however it
//! lost the machine (taken back, failed, replaced). Glave runs nothing again: the model does, if
//! it still wants it.

use std::sync::Arc;

use glave_ports::BoxFuture;
use glave_ports::computer::{ComputerError, Finished, Output};

use super::Waking;

/// What happened to a computer whose Pod has ended or gone, as the call cut off by it is told.
const MOVED: &str = "your computer moved to another machine";

/// What a computer may have run short of while a command ran (`specs/computers.md`, decision
/// 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Short {
    /// The command spent its run held at the CPU limit.
    Cpu,
    /// The computer went away as the command ran, perhaps killed for memory.
    Memory,
}

/// Grows the computer for what a command ran short of, where it did, and answers what to tell
/// whoever ran the command.
pub type Grow = Arc<dyn Fn(Short) -> BoxFuture<'static, Option<String>> + Send + Sync>;

/// Asks whether the computer, which a call found it could not reach, has lost its machine
/// (`specs/computers.md`, decision 16), rather than only the connection to it: what [`Waking`]
/// asks before it says why the call stopped.
pub type Lost = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

impl Waking {
    /// Whether the computer a call just found unreachable has lost its machine; never one with
    /// no machine of its own to lose.
    async fn machine_lost(&self) -> bool {
        match &self.machine {
            Some((_, lost)) => lost().await,
            None => false,
        }
    }

    /// `ran`, a command's run: one held at the CPU limit grows the computer, saying how in
    /// `output`, where whoever ran the command reads it; one cut off as the computer lost its
    /// machine says why instead of the error, and grows it if it was killed for memory.
    pub(super) async fn ran(
        &self,
        ran: Result<Finished, ComputerError>,
        output: &dyn Output,
    ) -> Result<Finished, ComputerError> {
        let Some((grow, _)) = &self.machine else {
            return ran;
        };
        match ran {
            Ok(finished) if finished.cpu_held => {
                if let Some(said) = grow(Short::Cpu).await {
                    output.write(&format!("\n{said}\n")).await;
                }
                Ok(finished)
            }
            Err(ComputerError::Unreachable(why)) => {
                self.distrust();
                if !self.machine_lost().await {
                    return Err(ComputerError::Unreachable(why));
                }
                Err(ComputerError::Failed(
                    grow(Short::Memory).await.unwrap_or_else(cut_off),
                ))
            }
            ran => ran,
        }
    }

    /// `answer`, a call's that reads or changes what is on the computer, counted as any call's
    /// on it: one cut off as it lost its machine says why instead of the error.
    pub(super) async fn told<T>(
        &self,
        answer: Result<T, ComputerError>,
    ) -> Result<T, ComputerError> {
        let lost = ComputerError::Failed(format!(
            "glave: {MOVED}, so this may not have finished. Your files are kept: check what it \
             changed, then try it again if it is still needed."
        ));
        self.unless_lost(answer, lost).await
    }

    /// `answer`, a call's about commands in the background, counted as any call's on the
    /// computer: with its machine lost they are gone with it, as a computer woken since would
    /// answer of them.
    pub(super) async fn gone_with_it<T>(
        &self,
        answer: Result<T, ComputerError>,
    ) -> Result<T, ComputerError> {
        let lost = ComputerError::NotFound("its computer started again".to_owned());
        self.unless_lost(answer, lost).await
    }

    /// `answer`, a call's on the computer, counted as any call's on it; `lost` in place of its
    /// error when it was cut off as the computer lost its machine.
    async fn unless_lost<T>(
        &self,
        answer: Result<T, ComputerError>,
        lost: ComputerError,
    ) -> Result<T, ComputerError> {
        let answer = self.done(answer);
        if matches!(answer, Err(ComputerError::Unreachable(_))) && self.machine_lost().await {
            return Err(lost);
        }
        answer
    }
}

/// What the call running a command is told when its computer lost its machine under it.
fn cut_off() -> String {
    format!(
        "glave: {MOVED}, and this command was stopped; glave did not run it again. Your files \
         are kept, but what it did may be half done. Run the command again if it is still \
         needed."
    )
}
