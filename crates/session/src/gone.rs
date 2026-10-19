//! A project whose folder isn't there anymore (`specs/session.md`, "Spaces and who may do
//! what"). The server notices when the project is listed and before anything would run in it,
//! and says so as a fact with its sentence: `space/list`'s `gone`, and `-32029` for what would
//! run there, so nothing starts and nothing is made in the folder's place. The person on this
//! computer mends it: `space/folder/set` gives the project its folder's new place, where its
//! threads work from then on; `space/remove` archives its threads and lists it no more.

use glave_event::{Principal, SpaceRemoved, ThreadId};
use glave_harness::{Change, change_standing};
use glave_ports::{
    BlobStore, Caller, EventLog, FolderKey, ModelProvider, Space, SpaceError, SpaceId, SpaceKind,
    Standing, ThreadQuery,
};

use crate::folders::only_here;
use crate::held::Slot;
use crate::protocol::{ApiError, GoneFolder, NewFolder, Reply, code};
use crate::providers::local;
use crate::server::{MAX_PAGE, Server};

/// What the person is told about a project whose folder isn't there anymore.
const GONE: &str = "This project's folder isn't there anymore.";

/// Refused: the project's folder isn't there anymore, so nothing runs in it.
pub(crate) fn folder_gone() -> ApiError {
    ApiError::new(code::NO_FOLDER, GONE)
}

/// Whether `error` says the project's folder isn't there anymore.
pub(crate) fn is_gone(error: &ApiError) -> bool {
    error.code == code::NO_FOLDER
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Of `spaces`, those `space/list` shows (a removed project is not one), and the projects
    /// among them whose folder isn't there anymore.
    pub(crate) async fn listed(
        &self,
        spaces: Vec<Space>,
    ) -> Result<(Vec<Space>, Vec<GoneFolder>), ApiError> {
        let (mut shown, mut gone) = (Vec::with_capacity(spaces.len()), Vec::new());
        for space in spaces {
            if space.kind == SpaceKind::Project {
                if self.removed(space.id).await? {
                    continue;
                }
                if self
                    .folder_found(space.id)
                    .await
                    .is_err_and(|e| is_gone(&e))
                {
                    gone.push(GoneFolder {
                        space: space.id,
                        message: GONE.to_owned(),
                    });
                }
            }
            shown.push(space);
        }
        Ok((shown, gone))
    }

    /// Answers `space/folder/set`: the folder at `path` on this computer becomes the folder of
    /// the project `space`, whose own isn't there anymore, and leads to it. Threads held idle
    /// in it are let go of, so each works where the folder is now the next time it is opened.
    pub(crate) async fn move_folder(
        &self,
        caller: &Caller,
        space: SpaceId,
        path: String,
    ) -> Result<Reply, ApiError> {
        let found = self
            .owned_here(caller, space, "gives a project its folder")
            .await?;
        let was = match self.folder_found(space).await {
            Err(error) if is_gone(&error) => self.folder_of(space).await?,
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(ApiError::new(
                    code::INVALID_PARAMS,
                    "this project's folder is still where it was",
                ));
            }
        };
        let (root, source) = self
            .make_folder(&found.name, NewFolder::Path { path })
            .await?;
        let mut keys = vec![FolderKey::Path(root.clone())];
        if let glave_event::FolderSource::Repository { url } = &source {
            keys.push(FolderKey::Remote(url.clone()));
        }
        // Its old place goes last: known, it leads to the project, so the new keys join it.
        if let Some(was) = was {
            keys.push(FolderKey::Path(was.checkout.path));
        }
        match self
            .inner
            .spaces
            .folder(self.inner.home.org, &keys, &found)
            .await
        {
            Ok(led) if led.id != space => {
                return Err(ApiError::new(
                    code::NOT_ALLOWED,
                    "that folder is already another project's",
                ));
            }
            // No key was known: the project keeps its folder in its ledger alone.
            Ok(_) | Err(SpaceError::Conflict(_)) => {}
            Err(error) => return Err(crate::spaces::failed(&error)),
        }
        let person = Principal::person(self.person_of(caller)?);
        let folder = self.set_folder(space, source, root, Some(person)).await?;
        self.let_go_in(space);
        Ok(Reply::Space {
            space: found,
            folder: Some(folder),
        })
    }

    /// Answers `space/remove`: every thread of the project `space` goes to the archive, their
    /// history kept, then the project is recorded removed. Refused while a turn runs in it.
    pub(crate) async fn remove_space(
        &self,
        caller: &Caller,
        space: SpaceId,
    ) -> Result<Reply, ApiError> {
        self.owned_here(caller, space, "removes a project").await?;
        let threads = self.active_threads(space).await?;
        if threads
            .iter()
            .any(|thread| matches!(self.lock().get(thread), Some(Slot::Busy { .. })))
        {
            return Err(ApiError::new(
                code::TURN_RUNNING,
                "something is running in this project: stop it first, then remove the project",
            ));
        }
        self.let_go_in(space);
        let scope = self.scope_in(space);
        for thread in threads {
            let now = self.inner.harness.clock().now();
            let random = self.inner.harness.entropy().bytes()?;
            let log = self.inner.harness.log();
            change_standing(log, &scope, thread, now, random, Change::Archive)
                .await
                .map_err(|error| crate::server::api(&error))?;
        }
        let by = Some(Principal::person(self.person_of(caller)?));
        self.write_space_ledger(space, |current| {
            Ok((!current.removed).then(|| SpaceRemoved { by: by.clone() }))
        })
        .await?;
        Ok(Reply::Done {})
    }

    /// Archives, unarchives or deletes `thread`, whose project's folder isn't there anymore,
    /// straight in the log, as the person on this computer: it cannot be opened to change it
    /// as a held thread is. Anyone else is told the folder is gone.
    pub(crate) async fn change_gone(
        &self,
        caller: &Caller,
        thread: ThreadId,
        change: Change,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(folder_gone());
        }
        let scope = self.scope_of(thread).await?;
        let now = self.inner.harness.clock().now();
        let random = self.inner.harness.entropy().bytes()?;
        change_standing(
            self.inner.harness.log(),
            &scope,
            thread,
            now,
            random,
            change,
        )
        .await
        .map_err(|error| crate::server::api(&error))?;
        Ok(Reply::Done {})
    }

    /// The project `space`, for `caller` on this computer, who owns it; refused otherwise, as
    /// only the person on this computer `does` what this is.
    async fn owned_here(
        &self,
        caller: &Caller,
        space: SpaceId,
        does: &str,
    ) -> Result<Space, ApiError> {
        if !local(caller) {
            return Err(only_here(&format!(
                "only the person on the computer glave runs on {does}"
            )));
        }
        let found = self.member_of(caller, space).await?;
        if found.kind != SpaceKind::Project {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                "only a project has a folder, and can be removed",
            ));
        }
        if found.owner != Principal::person(self.person_of(caller)?) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                format!("only its owner {does}"),
            ));
        }
        Ok(found)
    }

    /// Every thread of `space` that is neither archived nor deleted.
    async fn active_threads(&self, space: SpaceId) -> Result<Vec<ThreadId>, ApiError> {
        let scope = self.scope_in(space);
        let (mut threads, mut after) = (Vec::new(), None);
        loop {
            let query = ThreadQuery {
                standing: Standing::Active,
                limit: MAX_PAGE,
                after,
            };
            let views = self
                .inner
                .harness
                .log()
                .thread_views(&scope, &query)
                .await
                .map_err(|error| ApiError::new(code::INTERNAL, error.to_string()))?;
            let full = views.len() == MAX_PAGE;
            after = views.last().map(glave_ports::ThreadView::cursor);
            threads.extend(
                views
                    .into_iter()
                    .map(|view| view.thread)
                    .filter(|thread| !crate::ledger::is_ledger(*thread)),
            );
            if !full {
                return Ok(threads);
            }
        }
    }

    /// Lets go of every thread of `space` held idle, so each is read again from the log, and
    /// opened where its project's folder is now, the next time it is used.
    fn let_go_in(&self, space: SpaceId) {
        self.lock().retain(|_, slot| match slot {
            Slot::Idle { thread: held, .. } => held.scope.space != space,
            Slot::Busy { .. } => true,
        });
    }
}
