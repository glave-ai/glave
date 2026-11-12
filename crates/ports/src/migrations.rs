//! A store's schema carried forward (`specs/storage.md`, "Migrations"): the baseline a new store
//! is made at, then numbered migrations, each one step of SQL, appended and never edited once
//! shipped. Both stores decide here what a store at some version runs; each runs it in its own
//! database, in one transaction with the version it reaches.

use thiserror::Error;

/// What a migration does to what a store holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Adds tables, columns or indexes, or drops and makes again a table that holds only what
    /// is derived from the events (`threads`, `event_search`): nothing else a store holds
    /// changes.
    Adds,
    /// Changes or removes what a store holds, so SQLite copies its file first.
    Reshapes,
}

/// One step forward: the `n`th of a [`Schema`]'s migrations takes a store from version
/// `baseline + n - 1` to `baseline + n`.
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub change: Change,
    /// Statements run in the transaction that records the version it reaches.
    pub sql: &'static str,
}

/// A store's schema: what a new store is made with, and every migration since, in order.
#[derive(Clone, Copy, Debug)]
pub struct Schema<'a> {
    /// The version a new store is made at, and the oldest carried forward; at least 1, since 0
    /// is a store with nothing in it yet.
    pub baseline: u32,
    /// Every table as of `baseline`.
    pub made: &'static str,
    pub migrations: &'a [Migration],
}

/// Why a store is not opened, left as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum Refused {
    #[error("saved by a newer glave (schema {found}; this one reads up to {latest})")]
    Newer { found: u32, latest: u32 },
    #[error("saved by a glave from before stores were carried forward (schema {found})")]
    Older { found: u32 },
}

/// What a store runs, in one transaction, to reach the latest version.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Each step in order: the baseline for a new store, then the migrations it lacks. None
    /// when the store is at the latest version.
    pub steps: Vec<Step>,
    /// Whether a step changes or removes what the store already holds, so it is copied first.
    pub reshapes: bool,
}

/// One step of a [`Plan`]: its statements, and the version the store is at after them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub version: u32,
    pub sql: &'static str,
}

impl Schema<'_> {
    /// The version every store is brought to.
    pub fn latest(&self) -> u32 {
        let count = u32::try_from(self.migrations.len()).unwrap_or(u32::MAX);
        self.baseline.saturating_add(count)
    }

    /// What a store at version `found` (0 for one with nothing in it yet) runs to reach
    /// [`latest`](Self::latest), or why it is refused.
    pub fn plan(&self, found: u32) -> Result<Plan, Refused> {
        let latest = self.latest();
        if found > latest {
            return Err(Refused::Newer { found, latest });
        }
        if found == latest {
            return Ok(Plan::default());
        }
        if found != 0 && found < self.baseline {
            return Err(Refused::Older { found });
        }
        let made = Step {
            version: self.baseline,
            sql: self.made,
        };
        let numbered = self
            .migrations
            .iter()
            .zip(self.baseline.saturating_add(1)..);
        let lacking: Vec<(&Migration, u32)> =
            numbered.filter(|(_, version)| *version > found).collect();
        let steps = lacking.iter().map(|(migration, version)| Step {
            version: *version,
            sql: migration.sql,
        });
        Ok(Plan {
            steps: (found == 0)
                .then_some(made)
                .into_iter()
                .chain(steps)
                .collect(),
            // A new store holds nothing to lose.
            reshapes: found != 0
                && lacking
                    .iter()
                    .any(|(migration, _)| migration.change == Change::Reshapes),
        })
    }

    /// The first rule a migration breaks, saying which and why, if one does: none builds an
    /// index `CONCURRENTLY`, which cannot run in the transaction every migration runs in, and
    /// one that makes the threads view's table again (`threads`, or a new name for it) clears
    /// the setting that says which fold filled it (`threads_view`), so the store fills the new
    /// one from the log as it opens.
    pub fn broken_rule(&self) -> Option<String> {
        let mut numbered = self
            .migrations
            .iter()
            .zip(self.baseline.saturating_add(1)..);
        numbered.find_map(|(migration, version)| {
            let words = migration.sql.to_lowercase();
            let sql = words.split_whitespace().collect::<Vec<_>>().join(" ");
            if sql.contains("concurrently") {
                return Some(format!(
                    "{version} builds an index concurrently, which no transaction can"
                ));
            }
            let makes_threads = ["create table threads", "create table if not exists threads"]
                .iter()
                .any(|made| sql.contains(made));
            (makes_threads && !sql.contains("threads_view")).then(|| {
                format!("{version} makes the threads view's table and leaves `threads_view` set")
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATIONS: &[Migration] = &[
        Migration {
            change: Change::Adds,
            sql: "add",
        },
        Migration {
            change: Change::Reshapes,
            sql: "reshape",
        },
        Migration {
            change: Change::Adds,
            sql: "add again",
        },
    ];

    const SCHEMA: Schema<'static> = Schema {
        baseline: 5,
        made: "made",
        migrations: MIGRATIONS,
    };

    /// A plan of `steps`, each its SQL and the version it reaches.
    fn plan(steps: &[(&'static str, u32)], reshapes: bool) -> Plan {
        let steps = steps
            .iter()
            .map(|&(sql, version)| Step { version, sql })
            .collect();
        Plan { steps, reshapes }
    }

    #[test]
    fn each_migration_is_one_version_past_the_one_before() {
        assert_eq!(SCHEMA.latest(), 8);
        assert_eq!(SCHEMA.plan(7), Ok(plan(&[("add again", 8)], false)));
        assert_eq!(
            SCHEMA.plan(6),
            Ok(plan(&[("reshape", 7), ("add again", 8)], true))
        );
        assert_eq!(
            SCHEMA.plan(5),
            Ok(plan(&[("add", 6), ("reshape", 7), ("add again", 8)], true))
        );
    }

    #[test]
    fn a_new_store_is_made_at_the_baseline_and_brought_through_every_migration_uncopied() {
        let all = [("made", 5), ("add", 6), ("reshape", 7), ("add again", 8)];
        assert_eq!(SCHEMA.plan(0), Ok(plan(&all, false)));
    }

    #[test]
    fn a_store_at_the_latest_version_runs_nothing() {
        assert_eq!(SCHEMA.plan(8), Ok(Plan::default()));
    }

    #[test]
    fn a_store_newer_than_the_latest_or_older_than_the_baseline_is_refused() {
        assert_eq!(
            SCHEMA.plan(9),
            Err(Refused::Newer {
                found: 9,
                latest: 8
            })
        );
        assert_eq!(SCHEMA.plan(4), Err(Refused::Older { found: 4 }));
    }

    #[test]
    fn a_migration_that_builds_concurrently_or_remakes_the_threads_view_unfilled_breaks_a_rule() {
        let with = |sql: &'static str| {
            let one = [Migration {
                change: Change::Adds,
                sql,
            }];
            let schema = Schema {
                migrations: &one,
                ..SCHEMA
            };
            schema.broken_rule()
        };
        assert_eq!(SCHEMA.broken_rule(), None);
        assert_eq!(
            with("CREATE INDEX  CONCURRENTLY events_by_at ON events (at);"),
            Some("6 builds an index concurrently, which no transaction can".into())
        );
        assert_eq!(
            with("CREATE TABLE\n  threads_2 (thread text);"),
            Some("6 makes the threads view's table and leaves `threads_view` set".into())
        );
        let refilled = "CREATE TABLE threads_2 (thread text);
             DELETE FROM settings WHERE key = 'threads_view';";
        assert_eq!(with(refilled), None);
        assert_eq!(with("CREATE TABLE threadbare (x text);"), None);
    }

    #[test]
    fn a_schema_with_no_migrations_is_at_its_baseline() {
        let schema = Schema {
            migrations: &[],
            ..SCHEMA
        };
        assert_eq!(schema.latest(), 5);
        assert_eq!(schema.plan(0), Ok(plan(&[("made", 5)], false)));
        assert_eq!(schema.plan(5), Ok(Plan::default()));
    }
}
