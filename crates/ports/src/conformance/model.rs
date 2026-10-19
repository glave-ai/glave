//! What every model provider must do (`ModelProvider`), run against a fake server that speaks
//! its wire format: live deltas that add up to the finished answer, `NoOutput` only while
//! nothing has been shown (so asking again is safe), usage marked estimated when the provider
//! reported none, a timeout after a silence, and no secret in an error.

use glave_event::{Part, StopReason};
use tokio::runtime::{Handle, RuntimeFlavor};

use super::{Verdict, ensure};
use crate::model::{Completion, Delta, Message, ModelProvider, ModelRequest, ProviderError};
use crate::testing::fake_secret;

/// What the fake server behind a provider does with the one request it gets, in no
/// provider's terms. The test maps each to its own wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    /// Reasoning, then text, each streamed in the pieces given, then a normal end of the turn
    /// with usage.
    Answer {
        reasoning: &'static [&'static str],
        text: &'static [&'static str],
    },
    /// Text streamed in the pieces given and a normal end, with no usage reported.
    Unmetered { text: &'static [&'static str] },
    /// A stream that opens, then reports an error of a kind the protocol does not name,
    /// before any output.
    ErrorFirst,
    /// Text, then an error of a kind the protocol does not name.
    ErrorAfter { text: &'static str },
    /// Nothing: the request is held open and never answered.
    Silence,
    /// A refusal, HTTP 401, whose message repeats the key it was sent, as some providers' do.
    Refusal,
}

const REASONING: &[&str] = &["Think", "ing it ", "over."];
const TEXT: &[&str] = &["Hel", "lo, ", "there."];

/// Every [`ModelProvider`] must keep the promises its port makes. `serve` starts a fake server
/// playing a [`Script`] and returns a provider that asks it with `key`, and whatever keeps the
/// server running. Run on a current-thread runtime: the silence is waited out on a paused
/// clock, so the suite never sleeps.
pub async fn model_provider<P, G>(serve: impl Fn(Script, &str) -> Result<(P, G), String>) -> Verdict
where
    P: ModelProvider,
{
    let key = fake_secret("sk-");
    let play = |script| ask(&serve, script, &key);

    let answer = Script::Answer {
        reasoning: REASONING,
        text: TEXT,
    };
    let (result, deltas) = play(answer).await?;
    let completion = result.map_err(|e| format!("a streamed answer failed: {e}"))?;
    ensure(
        runs(&deltas) == shown(&completion)?,
        &format!(
            "the deltas {deltas:?} do not add up to the answer's parts {:?}",
            completion.parts
        ),
    )?;
    ensure(
        shown(&completion)?
            == [
                (Kind::Reasoning, REASONING.concat()),
                (Kind::Text, TEXT.concat()),
            ],
        &format!(
            "the answer's parts are not what was sent: {:?}",
            completion.parts
        ),
    )?;
    ensure(
        completion.stop == StopReason::EndTurn,
        &format!("a normal end did not end the turn: {:?}", completion.stop),
    )?;
    ensure(
        !completion.usage.estimated(),
        "usage the provider reported was marked estimated",
    )?;

    let (result, deltas) = play(Script::Unmetered { text: TEXT }).await?;
    let completion = result.map_err(|e| format!("an answer with no usage failed: {e}"))?;
    ensure(
        runs(&deltas) == shown(&completion)?,
        "the deltas of an answer with no usage do not add up to its parts",
    )?;
    ensure(
        completion.usage.estimated(),
        "usage the provider never reported was not marked estimated",
    )?;

    errors(&play).await?;
    secrets(&play, &key).await?;
    silence(&play).await
}

/// `NoOutput` before any output, and never after it.
async fn errors<F, Fut>(ask: &F) -> Verdict
where
    F: Fn(Script) -> Fut,
    Fut: Future<Output = Result<Asked, String>>,
{
    let (result, deltas) = ask(Script::ErrorFirst).await?;
    ensure(
        matches!(result, Err(ProviderError::NoOutput(_))),
        &format!("an error before any output was not NoOutput: {result:?}"),
    )?;
    ensure(deltas.is_empty(), "deltas were shown before NoOutput")?;

    let text = "Half an ans";
    let (result, deltas) = ask(Script::ErrorAfter { text }).await?;
    ensure(
        result.is_err(),
        "an error after output still gave a finished answer",
    )?;
    ensure(
        !matches!(result, Err(ProviderError::NoOutput(_))),
        "an error after output was NoOutput, so asking again would repeat what was shown",
    )?;
    ensure(
        runs(&deltas) == [(Kind::Text, text.to_owned())],
        &format!("the output before the error was not shown live: {deltas:?}"),
    )
}

/// A key the server repeats never shows in the error.
async fn secrets<F, Fut>(ask: &F, key: &str) -> Verdict
where
    F: Fn(Script) -> Fut,
    Fut: Future<Output = Result<Asked, String>>,
{
    let (result, _) = ask(Script::Refusal).await?;
    let Err(error) = result else {
        return Err("a refusal gave a finished answer".into());
    };
    ensure(
        matches!(error, ProviderError::Http { status: 401, .. }),
        &format!("a refusal was not HTTP 401: {error:?}"),
    )?;
    ensure(
        !error.to_string().contains(key) && !format!("{error:?}").contains(key),
        "the key the server repeated shows in the error",
    )
}

/// A server that sends nothing times out, on a paused clock that jumps to each deadline.
async fn silence<F, Fut>(ask: &F) -> Verdict
where
    F: Fn(Script) -> Fut,
    Fut: Future<Output = Result<Asked, String>>,
{
    ensure(
        Handle::try_current()
            .is_ok_and(|handle| handle.runtime_flavor() == RuntimeFlavor::CurrentThread),
        "the suite must run on a current-thread runtime, to wait out a silence on a paused clock",
    )?;
    tokio::time::pause();
    let asked = ask(Script::Silence).await;
    tokio::time::resume();
    let (result, deltas) = asked?;
    ensure(
        matches!(result, Err(ProviderError::Timeout { .. })),
        &format!("a server that sent nothing did not time out: {result:?}"),
    )?;
    ensure(
        result.as_ref().is_err_and(ProviderError::is_transient),
        "a silence is not worth another try",
    )?;
    ensure(deltas.is_empty(), "a silent server's request showed deltas")
}

type Asked = (Result<Completion, ProviderError>, Vec<Delta>);

/// Plays `script` and asks the provider once, keeping every delta.
async fn ask<P: ModelProvider, G>(
    serve: &impl Fn(Script, &str) -> Result<(P, G), String>,
    script: Script,
    key: &str,
) -> Result<Asked, String> {
    let (provider, _server) = serve(script, key)?;
    let mut deltas = Vec::new();
    let result = provider
        .complete(&request(), &mut |delta| deltas.push(delta))
        .await;
    Ok((result, deltas))
}

fn request() -> ModelRequest {
    ModelRequest {
        model: "conformance-model".into(),
        system: "Be brief.".into(),
        messages: vec![Message::User {
            parts: vec![Part::Text {
                text: "Hello?".into(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: crate::model::ToolChoice::Auto,
        max_output_tokens: None,
        purpose: crate::model::Purpose::Main,
        threads: Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Reasoning,
}

/// Deltas joined into runs of one kind.
fn runs(deltas: &[Delta]) -> Vec<(Kind, String)> {
    let mut runs: Vec<(Kind, String)> = Vec::new();
    for delta in deltas {
        let (kind, piece) = match delta {
            Delta::Text(text) => (Kind::Text, text),
            Delta::Reasoning(text) => (Kind::Reasoning, text),
        };
        match runs.last_mut() {
            Some((last, joined)) if *last == kind => joined.push_str(piece),
            _ => runs.push((kind, piece.clone())),
        }
    }
    runs
}

/// The readable parts of an answer: its text and reasoning, in order. Opaque reasoning shows
/// nothing; a file is never streamed, so an answer holding one breaks the rule.
fn shown(completion: &Completion) -> Result<Vec<(Kind, String)>, String> {
    let mut shown = Vec::new();
    for part in &completion.parts {
        match part {
            Part::Text { text } => shown.push((Kind::Text, text.clone())),
            Part::Reasoning {
                text: Some(text), ..
            } => shown.push((Kind::Reasoning, text.clone())),
            Part::Reasoning { text: None, .. } => {}
            Part::File { .. } => return Err("an answer held a file no delta showed".into()),
        }
    }
    Ok(shown)
}
