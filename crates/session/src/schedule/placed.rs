//! The store's index of where schedules are, so every server that fires schedules fires
//! everyone's, though no one asked it for anything since it started. Each organization's own
//! stream lists its projects where someone made a schedule (`schedule.placed`), so no
//! organization's are listed under another's; one stream in the server's own organization lists
//! the organizations where someone did (`schedule.organization`), naming no project. Each is
//! listed once.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::PoisonError;

use glave_event::{
    At, Body, EventId, Header, Payload, RawEvent, ScheduleOrganization, SchedulePlaced, Seq, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Located, ModelProvider, OrgId, ProjectId, Scope};

use crate::protocol::ApiError;
use crate::server::Server;

/// The most projects, and the most organizations, a server reads from the index: past it, a
/// project's schedules fire once its person asks this server for anything, as before the index.
const MOST_PLACED: usize = 100_000;

/// One stream of the index, as far as this server has read it: what it lists, in order.
#[derive(Debug)]
struct Listed<T> {
    next: u64,
    items: Vec<T>,
    held: HashSet<T>,
}

impl<T> Default for Listed<T> {
    fn default() -> Self {
        Self {
            next: 0,
            items: Vec::new(),
            held: HashSet::new(),
        }
    }
}

impl<T: Copy + Eq + Hash> Listed<T> {
    /// Takes in the event at the next `seq`, listing what `read` finds in it while there is
    /// `room`.
    fn apply(&mut self, raw: RawEvent, room: bool, read: impl Fn(Body) -> Option<T>) {
        let Some((_, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        if let Some(item) = read(body)
            && room
            && self.held.insert(item)
        {
            self.items.push(item);
        }
    }

    /// The `seq` the next event takes, when `item` is not listed yet and there is `room` for
    /// it: past the most read, nothing more is listed, since no server would read it.
    fn absent(&self, item: T, room: bool) -> Option<u64> {
        (room && !self.held.contains(&item)).then_some(self.next.max(1))
    }
}

/// The index as far as this server has read it, and how many ticks of the scheduler read it.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    organizations: Listed<OrgId>,
    projects: HashMap<OrgId, Listed<ProjectId>>,
    ticks: usize,
}

impl Placed {
    /// Whether what `stream` lists has room for one more, as read so far: organizations, and
    /// projects across every organization, up to [`MOST_PLACED`] each.
    fn room(&self, stream: Stream) -> bool {
        let listed = match stream {
            Stream::Organizations => self.organizations.items.len(),
            Stream::Projects(_) => self.projects.values().map(|it| it.items.len()).sum(),
        };
        listed < MOST_PLACED
    }

    fn apply(&mut self, stream: Stream, raw: RawEvent) {
        let room = self.room(stream);
        match stream {
            Stream::Organizations => {
                self.organizations.apply(raw, room, |body| match body {
                    Body::ScheduleOrganization(listed) => Some(OrgId::from_uuid(listed.org)),
                    _ => None,
                });
            }
            Stream::Projects(org) => {
                let listed = self.projects.entry(org).or_default();
                listed.apply(raw, room, |body| match body {
                    Body::SchedulePlaced(placed) => Some(ProjectId::from_uuid(placed.project)),
                    _ => None,
                });
            }
        }
    }

    fn next(&self, stream: Stream) -> u64 {
        match stream {
            Stream::Organizations => self.organizations.next,
            Stream::Projects(org) => self.projects.get(&org).map_or(0, |listed| listed.next),
        }
    }
}

/// A stream of the index: the store's list of organizations, or one organization's projects.
#[derive(Debug, Clone, Copy)]
enum Stream {
    Organizations,
    Projects(OrgId),
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Every project the index lists, read up to now. An organization whose own index cannot
    /// be read now is read again next time.
    pub(crate) async fn placed(&self) -> Result<Vec<Located>, ApiError> {
        self.catch_up_placed(Stream::Organizations).await?;
        let organizations = self.held_placed(|held| held.organizations.items.clone());
        let mut placed = Vec::new();
        for org in organizations {
            if let Err(error) = self.catch_up_placed(Stream::Projects(org)).await {
                tracing::warn!(%error, %org, "could not read where the organization's schedules are");
                continue;
            }
            let projects = self.held_placed(|held| {
                (held.projects.get(&org)).map_or_else(Vec::new, |listed| listed.items.clone())
            });
            placed.extend(projects.into_iter().map(|project| Located { org, project }));
        }
        Ok(placed)
    }

    /// Whether the project `at`'s organization's index lists it, read up to now: whether anyone
    /// made a schedule there.
    pub(crate) async fn has_schedules(&self, at: Located) -> Result<bool, ApiError> {
        self.catch_up_placed(Stream::Projects(at.org)).await?;
        Ok(self.held_placed(|held| {
            (held.projects.get(&at.org)).is_some_and(|listed| listed.held.contains(&at.project))
        }))
    }

    /// Which organization, counting from the first, this tick of the scheduler starts from:
    /// one further on each time, so none waits behind the others every time.
    pub(crate) fn next_turn(&self) -> usize {
        self.held_placed(|held| {
            let turn = held.ticks;
            held.ticks = turn.wrapping_add(1);
            turn
        })
    }

    /// Lists the project `at` in its organization's index, and the organization in the store's
    /// list, unless they are listed already: the organization first, so a listed project is
    /// always found.
    pub(crate) async fn place_schedules(&self, at: Located) -> Result<(), ApiError> {
        let organization = ScheduleOrganization {
            org: at.org.as_uuid(),
        };
        let listed = |held: &Placed| {
            let room = held.room(Stream::Organizations);
            held.organizations.absent(at.org, room)
        };
        self.list_once(Stream::Organizations, &organization, listed)
            .await?;
        let placed = SchedulePlaced {
            project: at.project.as_uuid(),
        };
        let listed = |held: &Placed| {
            let room = held.room(Stream::Projects(at.org));
            (held.projects.get(&at.org))
                .map_or(room.then_some(1), |listed| listed.absent(at.project, room))
        };
        self.list_once(Stream::Projects(at.org), &placed, listed)
            .await
    }

    /// Appends `payload` to `stream`, after anything others wrote meanwhile, unless `absent`
    /// says what it lists is there already.
    async fn list_once<T: Payload>(
        &self,
        stream: Stream,
        payload: &T,
        absent: impl Fn(&Placed) -> Option<u64>,
    ) -> Result<(), ApiError> {
        let (scope, index) = (self.scope_of_stream(stream), Reserved::Scheduled.id());
        let log = self.inner.harness.log();
        let absent = &absent;
        let appended = append_retrying(log, &scope, index, || async move {
            self.catch_up_placed(stream).await?;
            let Some(next) = self.held_placed(|held| absent(held)) else {
                return Ok::<_, ApiError>(None);
            };
            let now = self.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
                thread: index,
                seq: Seq::new(next)?,
                at: At::from(now),
            };
            Ok(Some((vec![encode(header, payload)?], ())))
        })
        .await?;
        self.held_placed(|held| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                held.apply(stream, raw);
            }
        });
        Ok(())
    }

    /// Reads whatever was added to `stream` since this server last looked.
    async fn catch_up_placed(&self, stream: Stream) -> Result<(), ApiError> {
        let scope = self.scope_of_stream(stream);
        let next = self.held_placed(|held| held.next(stream).max(1));
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::Scheduled.id(), after, |page| {
            self.held_placed(|held| {
                for raw in page {
                    held.apply(stream, raw);
                }
            });
        })
        .await?;
        Ok(())
    }

    /// Where `stream` is kept, glave's own: the list in the server's own organization, each
    /// organization's projects in its own.
    fn scope_of_stream(&self, stream: Stream) -> Scope {
        match stream {
            Stream::Organizations => Scope::of_organization(self.inner.own),
            Stream::Projects(org) => Scope::of_organization(org),
        }
    }

    fn held_placed<R>(&self, look: impl FnOnce(&mut Placed) -> R) -> R {
        let mut held = (self.inner.placed)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut held)
    }
}
