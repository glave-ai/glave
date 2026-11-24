//! A person's Library over the session API (`specs/session.md`, "Library"): what glave made
//! for them, kept beyond any chat or computer. Each item is in the Library ledger of the
//! project it was made in, its file a blob of that project's, held under a holder of its own;
//! `library/list` reads the ledgers of every project the caller owns, so no one sees another's;
//! `library/read` and `library/remove` name the item's project, which the caller must own. An
//! item another person has, or none has, is answered alike: "no longer here".

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use glave_event::{
    AssetId, At, Body, LibraryKept, LibraryRemoved, Payload as _, Principal, SharedFrom, ThreadId,
    decode,
};
use glave_ports::stream::{Reserved, catch_up};
use glave_ports::{
    BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope, Standing, title_of,
};

use crate::files::not_here;
use crate::projects::MOST_PROJECTS;
use crate::protocol::{ApiError, LibraryChat, LibraryItem, Reply, Told};
use crate::server::Server;

/// The most items `library/list` answers with, newest first.
const MOST_ITEMS: usize = 1_000;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The scopes of every project `caller` owns in the organization the request works in.
    pub(crate) async fn libraries(&self, caller: &Caller) -> Result<Vec<Scope>, ApiError> {
        let org = self.org_of(caller);
        let person = self.person_of(caller)?;
        let projects = self
            .inner
            .projects
            .list(org, &Principal::person(person), MOST_PROJECTS)
            .await?;
        Ok(projects
            .iter()
            .filter_map(|project| Scope::of_owner(org, project, person))
            .collect())
    }

    /// The items `scope`'s Library holds now, oldest first, each with when it was kept.
    async fn library_in(&self, scope: &Scope) -> Result<Vec<(At, LibraryKept)>, ApiError> {
        let mut items = BTreeMap::new();
        let log = self.inner.harness.log();
        catch_up(log, scope, Reserved::Library.id(), None, |page| {
            for raw in page {
                let at = raw.at;
                match decode(raw).map(|event| event.body) {
                    Ok(Body::LibraryKept(kept)) => {
                        items.insert(kept.asset, (at, kept));
                    }
                    Ok(Body::LibraryRemoved(removed)) => {
                        items.remove(&removed.asset);
                    }
                    _ => {}
                }
            }
        })
        .await?;
        Ok(items.into_values().collect())
    }

    /// The item `asset` of the Library of `project`, when `caller` owns it, and its scope; a
    /// project they don't own is "no project", as one that does not exist.
    pub(crate) async fn item(
        &self,
        caller: &Caller,
        project: ProjectId,
        asset: AssetId,
    ) -> Result<(Scope, LibraryKept), ApiError> {
        let (_, scope) = self.authorized(caller, project).await?;
        let found = self.library_in(&scope).await?;
        let kept = found.into_iter().find(|(_, kept)| kept.asset == asset);
        let (_, kept) = kept.ok_or_else(not_here)?;
        Ok((scope, kept))
    }

    /// Answers `library/list`: the items of every project `caller` owns, newest first, each
    /// with the chat it was made in while that chat is there.
    pub(crate) async fn list_library(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let mut found = Vec::new();
        for scope in self.libraries(caller).await? {
            let items = self.library_in(&scope).await?;
            found.extend(items.into_iter().map(|(at, kept)| (scope, at, kept)));
        }
        found.sort_by_key(|(_, at, kept)| std::cmp::Reverse((*at, kept.asset)));
        found.truncate(MOST_ITEMS);
        let mut chats: HashMap<ThreadId, Option<LibraryChat>> = HashMap::new();
        let mut shared: HashMap<Scope, HashMap<SharedFrom, Told>> = HashMap::new();
        let mut items = Vec::with_capacity(found.len());
        for (scope, made_at, kept) in found {
            let chat = if let Some(chat) = chats.get(&kept.thread) {
                chat.clone()
            } else {
                let chat = self.chat(&scope, &kept).await?;
                chats.insert(kept.thread, chat.clone());
                chat
            };
            if let Entry::Vacant(vacant) = shared.entry(scope) {
                vacant.insert(self.shared_sentences(&scope).await?);
            }
            let from = SharedFrom::Item { asset: kept.asset };
            let said = shared.get(&scope).and_then(|said| said.get(&from)).cloned();
            items.push(LibraryItem {
                asset: kept.asset,
                kind: kept.kind,
                name: kept.name,
                file_name: kept.file_name,
                media_type: kept.blob.media_type,
                bytes: kept.blob.bytes,
                project: scope.project(),
                made_at,
                chat,
                shared: said,
            });
        }
        Ok(Reply::Library { items })
    }

    /// The chat `kept` was made in, with its title, while it is there: none once deleted.
    async fn chat(
        &self,
        scope: &Scope,
        kept: &LibraryKept,
    ) -> Result<Option<LibraryChat>, ApiError> {
        let log = self.inner.harness.log();
        let Some(view) = log.thread_view(scope, kept.thread).await? else {
            return Ok(None);
        };
        if view.standing == Standing::Deleted {
            return Ok(None);
        }
        let title = self.event_at(scope, view.thread, view.title_seq).await?;
        Ok(Some(LibraryChat {
            thread: view.thread,
            title: title.as_ref().and_then(title_of),
        }))
    }

    /// Answers `library/read`: the piece of `asset`'s file that starts at `offset`.
    pub(crate) async fn read_library(
        &self,
        caller: &Caller,
        (project, asset): (ProjectId, AssetId),
        offset: u64,
    ) -> Result<Reply, ApiError> {
        let (scope, kept) = self.item(caller, project, asset).await?;
        self.read_piece(&scope, &kept.blob.sha256, offset).await
    }

    /// Answers `library/remove`: lets go of `asset`'s file, then takes it out of the caller's
    /// Library; the chat it was made in keeps its own hold. Let go of first, so a failure
    /// either way leaves the item listed, and removing it again finishes the work.
    pub(crate) async fn remove_from_library(
        &self,
        caller: &Caller,
        project: ProjectId,
        asset: AssetId,
    ) -> Result<Reply, ApiError> {
        let (scope, kept) = self.item(caller, project, asset).await?;
        self.inner
            .harness
            .blobs()
            .release(&scope, kept.holder)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        let kinds = [LibraryKept::TYPE, LibraryRemoved::TYPE];
        let removed = LibraryRemoved { asset };
        self.keep_latest(&scope, Reserved::Library.id(), &kinds, &removed)
            .await?;
        // Its share stops with it: nothing shared outlives the place its owner sees it.
        let item = SharedFrom::Item { asset };
        self.stop_shares(&scope, |of| *of == item).await?;
        Ok(Reply::Done {})
    }
}
