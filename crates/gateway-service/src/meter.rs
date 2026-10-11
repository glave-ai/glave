//! Metering: every call a caller makes is recorded, content-free, in a stream of their own in
//! the gateway's scope of the log, whose id is their person id. A model call is a
//! `model.responded`, a catalog tool call a `gateway.tool.called`. Cost reports are views over
//! these streams; and when a caller first comes back in a day, what they already spent that
//! day is read back from theirs, so a restart never resets a daily budget.
//!
//! Any number of gateway processes may append to one stream: an append that meets another's
//! `(thread, seq)` reads what was written and tries again after it.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::{Mutex, MutexGuard, PoisonError};

use glave_event::{
    At, Body, Currency, EventId, Header, Payload, PrincipalId, RawEvent, Seq, ThreadId, decode,
    encode,
};
use glave_gateway::{Budget, Caller, Gateway, Limit};
use glave_ports::{Clock, Entropy};
use glave_ports::{EventLog, LogError, Scope};

/// Events read per page.
const PAGE: usize = 500;
/// Appends tried before giving up on a stream other writers keep moving.
const ATTEMPTS: usize = 8;
/// Callers whose streams are followed at once. Past this, one is forgotten and read again
/// when it comes back; its budget for the day may then count twice, never too little.
const MOST_CALLERS: usize = 100_000;
/// Seconds in a day: budgets run by the UTC day.
const DAY: i64 = 86_400;

/// What is known of one caller's stream.
#[derive(Debug, Default, Clone)]
struct Stream {
    /// The `seq` the next event will take, once read; 0 before.
    next: u64,
    /// The day the totals below are for.
    day: u32,
    /// Tokens in and out that day.
    tokens: u64,
    /// Money spent that day, by currency, in millionths.
    money: BTreeMap<Currency, u64>,
    /// The day their budget account was given what the log says they spent before.
    seeded: Option<u32>,
}

impl Stream {
    fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet.
    fn apply(&mut self, raw: RawEvent) {
        if raw.seq.get() != self.next_seq() {
            return;
        }
        self.next = self.next_seq().saturating_add(1);
        let day = day_of(raw.at.timestamp());
        let Ok(event) = decode(raw) else {
            return;
        };
        if let Body::ModelResponded(responded) = event.body {
            if day != self.day {
                if day < self.day {
                    return;
                }
                (self.day, self.tokens) = (day, 0);
                self.money.clear();
            }
            let counts = responded.usage.counts();
            self.tokens = self
                .tokens
                .saturating_add(counts.input.saturating_add(counts.output));
            if let Some(cost) = responded.cost {
                let micros = u64::try_from(cost.amount.micros).unwrap_or(0);
                let spent = self.money.entry(cost.amount.currency).or_default();
                *spent = spent.saturating_add(micros);
            }
        }
    }

    /// What the stream says was spent on `day`, in `limit`'s unit.
    fn spent(&self, day: u32, limit: Limit) -> u64 {
        if day != self.day {
            return 0;
        }
        match limit {
            Limit::Tokens(_) => self.tokens,
            Limit::Money(cap) => self.money.get(&cap.currency).copied().unwrap_or(0),
        }
    }
}

/// The day `at` falls on, counted from 1970 in UTC.
fn day_of(at: jiff::Timestamp) -> u32 {
    u32::try_from(at.as_second().div_euclid(DAY)).unwrap_or(0)
}

/// Each caller's stream in the log.
pub(crate) struct Meter<L> {
    log: L,
    scope: Scope,
    clock: Box<dyn Clock>,
    entropy: Box<dyn Entropy>,
    streams: Mutex<HashMap<PrincipalId, Stream>>,
}

impl<L: EventLog> Meter<L> {
    pub(crate) fn new(
        log: L,
        scope: Scope,
        clock: Box<dyn Clock>,
        entropy: Box<dyn Entropy>,
    ) -> Self {
        Self {
            log,
            scope,
            clock,
            entropy,
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// Today, as budgets count days.
    pub(crate) fn today(&self) -> u32 {
        day_of(self.clock.now())
    }

    pub(crate) fn now(&self) -> jiff::Timestamp {
        self.clock.now()
    }

    /// A fresh id for an answer, in hex.
    pub(crate) fn fresh_id(&self) -> String {
        self.entropy
            .bytes()
            .iter()
            .fold(String::new(), |mut id, byte| {
                let _ = write!(id, "{byte:02x}");
                id
            })
    }

    fn streams(&self) -> MutexGuard<'_, HashMap<PrincipalId, Stream>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stream(&self, person: PrincipalId) -> Stream {
        self.streams().get(&person).cloned().unwrap_or_default()
    }

    /// Applies `events` to `person`'s stream as known here.
    fn take_in(&self, person: PrincipalId, events: Vec<RawEvent>) {
        let mut streams = self.streams();
        if streams.len() >= MOST_CALLERS && !streams.contains_key(&person) {
            let forgotten = streams.keys().next().copied();
            if let Some(forgotten) = forgotten {
                streams.remove(&forgotten);
            }
        }
        let stream = streams.entry(person).or_default();
        for raw in events {
            stream.apply(raw);
        }
    }

    /// Reads whatever was added to `person`'s stream since this process last looked.
    async fn catch_up(&self, person: PrincipalId) -> Result<(), String> {
        let thread = stream_id(person)?;
        loop {
            let next = self.stream(person).next_seq();
            let after = Seq::new(next.saturating_sub(1)).ok();
            let page = self
                .log
                .read(&self.scope, thread, after, PAGE)
                .await
                .map_err(|error| error.to_string())?;
            let full = page.len() == PAGE;
            self.take_in(person, page);
            if !full {
                return Ok(());
            }
        }
    }

    /// Before `person`'s first call of `day` here: counts what their stream says they spent
    /// that day already in their budget, so a restart does not reset it.
    pub(crate) async fn seed(
        &self,
        gateway: &Gateway,
        person: PrincipalId,
        day: u32,
    ) -> Result<(), String> {
        let Some(limit) = gateway.budgets().person else {
            return Ok(());
        };
        if self.stream(person).seeded == Some(day) {
            return Ok(());
        }
        self.catch_up(person).await?;
        let mut streams = self.streams();
        let stream = streams.entry(person).or_default();
        // Another call may have seeded it meanwhile; only one counts it.
        if stream.seeded != Some(day) {
            stream.seeded = Some(day);
            let caller = Caller {
                person,
                period: day,
            };
            gateway.add_spent(Budget::Caller(caller), stream.spent(day, limit));
        }
        Ok(())
    }

    /// Appends `payload` to `person`'s stream, after anything others wrote meanwhile.
    pub(crate) async fn record<T: Payload>(
        &self,
        person: PrincipalId,
        payload: &T,
    ) -> Result<(), String> {
        let thread = stream_id(person)?;
        for _ in 0..ATTEMPTS {
            self.catch_up(person).await?;
            let now = self.clock.now();
            let seq = Seq::new(self.stream(person).next_seq()).map_err(|e| e.to_string())?;
            let header = Header {
                id: EventId::new(now, self.entropy.bytes()).map_err(|e| e.to_string())?,
                thread,
                seq,
                at: At::from(now),
            };
            let raw = encode(header, payload).map_err(|e| e.to_string())?;
            match self
                .log
                .append(&self.scope, std::slice::from_ref(&raw))
                .await
            {
                Ok(()) => {
                    self.take_in(person, vec![raw]);
                    return Ok(());
                }
                Err(LogError::Conflict { .. } | LogError::Gap { .. }) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Err(format!(
            "the gateway stream of {person} kept changing; the call was not recorded"
        ))
    }
}

/// The stream a person's calls are recorded in: the id of the person themself, in the
/// gateway's own scope, where no thread lives.
pub fn stream_id(person: PrincipalId) -> Result<ThreadId, String> {
    ThreadId::try_from(person.as_uuid()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_utc_days_since_1970() {
        let at = |text: &str| day_of(text.parse().unwrap());
        assert_eq!(at("1970-01-01T23:59:59Z"), 0);
        assert_eq!(at("1970-01-02T00:00:00Z"), 1);
        assert_eq!(at("2026-10-11T12:00:00Z"), at("2026-10-11T00:00:00Z"));
    }
}
