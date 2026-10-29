//! How big a person's computer is (`specs/computers.md`, decision 8): it wakes at the smallest
//! size GKE Autopilot schedules, a quarter of a CPU and 512 MiB, requested and limited alike,
//! and grows when a command needs more, doubling what it ran short of up to a ceiling: memory
//! after a command was killed for it, CPU after a command spent its run held at the limit. Each
//! wake starts small again, but the one a computer too big to grow in place sleeps for.

use std::sync::{Mutex, PoisonError};

use crate::Short;

/// A computer's CPU, in thousandths of one, and its memory, in MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Size {
    pub(crate) cpu_millis: u32,
    pub(crate) memory_mib: u32,
}

/// Where every person's computer wakes: a quarter of a CPU and 512 MiB.
pub(crate) const SMALLEST: Size = Size {
    cpu_millis: 250,
    memory_mib: 512,
};

/// The largest a person's computer grows where the organization's limits name none: 8 CPUs and
/// 16 GiB, Glave Cloud's personal plan's.
pub(crate) const CEILING: Size = Size {
    cpu_millis: 8_000,
    memory_mib: 16_384,
};

impl Size {
    /// This, with what ran `short` doubled, within `ceiling`; none when it cannot grow.
    pub(crate) fn grown(self, short: Short, ceiling: Self) -> Option<Self> {
        let double = |now: u32, most: u32, short: bool| {
            if short {
                now.saturating_mul(2).min(most).max(now)
            } else {
                now
            }
        };
        let grown = Self {
            cpu_millis: double(self.cpu_millis, ceiling.cpu_millis, short == Short::Cpu),
            memory_mib: double(self.memory_mib, ceiling.memory_mib, short == Short::Memory),
        };
        (grown != self).then_some(grown)
    }

    /// What it has of what ran `short`, in words: `1 GiB`, `0.5 CPUs`.
    pub(crate) fn said(self, short: Short) -> String {
        match short {
            Short::Memory => memory_words(self.memory_mib),
            Short::Cpu => cpu_words(self.cpu_millis),
        }
    }
}

fn memory_words(mib: u32) -> String {
    if mib.is_multiple_of(1_024) {
        format!("{} GiB", mib / 1_024)
    } else {
        format!("{mib} MiB")
    }
}

fn cpu_words(millis: u32) -> String {
    let (whole, part) = (millis / 1_000, millis % 1_000);
    if part == 0 {
        let plural = if whole == 1 { "" } else { "s" };
        return format!("{whole} CPU{plural}");
    }
    let part = format!("{part:03}");
    format!("{whole}.{} CPUs", part.trim_end_matches('0'))
}

/// One person's computer's size: what it is now, and what its next wake starts at.
#[derive(Debug)]
pub(crate) struct Sizing {
    now: Mutex<Size>,
    next: Mutex<Option<Size>>,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            now: Mutex::new(SMALLEST),
            next: Mutex::new(None),
        }
    }
}

impl Sizing {
    /// The size a wake starts at now: the smallest, or what a computer too big to grow in place
    /// slept to wake at, once.
    pub(crate) fn waking(&self) -> Size {
        let size = self
            .next
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or(SMALLEST);
        self.set(size);
        size
    }

    /// The size it is now.
    pub(crate) fn now(&self) -> Size {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts it grown in place to `size`.
    pub(crate) fn set(&self, size: Size) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = size;
    }

    /// Has its next wake start at `size`.
    pub(crate) fn wake_at(&self, size: Size) {
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) = Some(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMORY: Short = Short::Memory;
    const CPU: Short = Short::Cpu;

    #[test]
    fn a_computer_grows_by_doubling_what_it_ran_short_of_up_to_its_ceiling() {
        let grown = SMALLEST.grown(MEMORY, CEILING).unwrap();
        assert_eq!(
            grown,
            Size {
                cpu_millis: 250,
                memory_mib: 1_024
            }
        );
        assert_eq!(grown.said(MEMORY), "1 GiB");
        let faster = SMALLEST.grown(CPU, CEILING).unwrap();
        assert_eq!(faster.cpu_millis, 500);
        assert_eq!(faster.said(CPU), "0.5 CPUs");
        let near = Size {
            cpu_millis: 6_000,
            memory_mib: 12_288,
        };
        let capped = near.grown(MEMORY, CEILING).unwrap();
        assert_eq!(capped.memory_mib, 16_384, "never past the ceiling");
        assert_eq!(
            CEILING.grown(MEMORY, CEILING),
            None,
            "at the ceiling, nothing grows"
        );
        assert_eq!(cpu_words(1_000), "1 CPU");
        assert_eq!(cpu_words(250), "0.25 CPUs");
        assert_eq!(memory_words(512), "512 MiB");
    }

    #[test]
    fn each_wake_starts_small_but_the_one_a_computer_slept_to_grow_for() {
        let sizing = Sizing::default();
        let big = SMALLEST.grown(MEMORY, CEILING).unwrap();
        sizing.set(big);
        assert_eq!(sizing.waking(), SMALLEST);
        sizing.wake_at(big);
        assert_eq!(sizing.waking(), big);
        assert_eq!(sizing.now(), big);
        assert_eq!(sizing.waking(), SMALLEST, "once");
    }
}
