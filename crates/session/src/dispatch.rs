//! The one dispatch: every request, from any transport, is answered by [`Server::call`],
//! either at once with a reply or with a turn to follow. A transport carries the answer; it
//! never decides what a method does.

use glave_harness::{Caller, Change, Control, DEFAULT_BUDGET, Goal};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::credentials::NewCredential;
use crate::modes::Choice;
use crate::protocol::{ApiError, Reply, Request, ServerInfo, code};
use crate::providers::NewProvider;
use crate::schedule::NewSchedule;
use crate::server::Server;
use crate::tasks::Work;
use crate::turn::TurnStream;

/// What a request is answered with.
#[expect(
    clippy::large_enum_variant,
    reason = "one per request, moved once: boxing would only add an allocation"
)]
pub enum Answered {
    /// The reply, or why there is none.
    Reply(Result<Reply, ApiError>),
    /// A turn, or a goal's turns, that someone follows: `turn/start` and `goal/start` without
    /// `background`. Its notifications, then how it ended.
    Stream(TurnStream),
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `request` from `caller`, the person the transport verified. A request on a
    /// thread, or on the server's tasks, schedules or memory, is refused with `-32012` unless
    /// the caller works in that space.
    pub async fn call(&self, caller: &Caller, request: Request) -> Answered {
        let span = crate::trace::span(&request);
        tracing::Instrument::instrument(self.answer(caller, request), span).await
    }

    /// Answers one request from `caller`; a turn is answered once it has ended.
    pub async fn request_as(&self, caller: &Caller, request: Request) -> Result<Reply, ApiError> {
        match self.call(caller, request).await {
            Answered::Reply(reply) => reply,
            Answered::Stream(mut stream) => stream.finish().await.map(|turn| Reply::Turn { turn }),
        }
    }

    /// Answers one request from the local person; see [`Server::request_as`].
    pub async fn request(&self, request: Request) -> Result<Reply, ApiError> {
        self.request_as(&Caller::default(), request).await
    }

    /// Starts `request`, a turn or a goal someone follows, and streams it. Any other request
    /// is answered at once, so it is refused here.
    pub async fn start_streaming(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<TurnStream, ApiError> {
        let method = request.method();
        match self.call(caller, request).await {
            Answered::Stream(stream) => Ok(stream),
            Answered::Reply(reply) => Err(reply.err().unwrap_or_else(|| {
                ApiError::new(
                    code::INVALID_PARAMS,
                    format!("{method} answered at once: only a turn or goal it follows streams"),
                )
            })),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per method: the API's one dispatch"
    )]
    async fn answer(&self, caller: &Caller, request: Request) -> Answered {
        if let Err(refused) = self.allow(caller, &request).await {
            return Answered::Reply(Err(refused));
        }
        // A turn joins the caller's trace, when they sent one.
        let traced = &Caller {
            trace: crate::trace::trace_of(&request).or_else(|| caller.trace.clone()),
            ..caller.clone()
        };
        let streamed = |started: Result<TurnStream, ApiError>| match started {
            Ok(stream) => Answered::Stream(stream),
            Err(refused) => Answered::Reply(Err(refused)),
        };
        Answered::Reply(match request {
            Request::Discover {} => Ok(Reply::Server(ServerInfo {
                name: "glave".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                protocols: vec![crate::protocol::PROTOCOL.to_owned()],
                approval: self.inner.harness.settings().approval,
                approvals: self.inner.modes.clone(),
            })),
            Request::ModelList {} => Ok(self.list_models()),
            Request::ThreadStart {
                computer: Some(computer),
                ..
            } => Err(ApiError::new(
                code::NOT_AVAILABLE,
                format!(
                    "threads on a computer of your own (computer {computer}) are not available \
                     yet: start the thread without `computer`"
                ),
            )),
            Request::ThreadStart {
                space,
                worktree,
                background,
                computer: None,
                model,
                approval,
            } => {
                let choice = Choice { model, approval };
                self.start_thread(caller, space, worktree, background, &choice)
                    .await
            }
            Request::ThreadList {
                space,
                limit,
                show,
                cursor,
            } => self.list_threads(caller, space, limit, show, cursor).await,
            Request::ThreadGet { thread } => self.get_thread(thread).await,
            Request::ThreadRename { thread, title } => self.rename(caller, thread, title).await,
            Request::ThreadArchive { thread } => self.change(caller, thread, Change::Archive).await,
            Request::ThreadUnarchive { thread } => {
                self.change(caller, thread, Change::Unarchive).await
            }
            Request::ThreadDelete { thread } => self.change(caller, thread, Change::Delete).await,
            Request::ThreadEvents {
                thread,
                after,
                limit,
            } => self.events(thread, after, limit).await,
            Request::ThreadCompact { thread } => self.compact(thread).await,
            Request::TurnStart {
                thread,
                prompt,
                background: false,
                budget,
                model,
                approval,
                meta,
            } => {
                if budget.is_some() {
                    return Answered::Reply(Err(unenforced_budget()));
                }
                let key = meta.idempotency_key.as_ref();
                let choice = Choice { model, approval };
                return streamed(
                    self.start_turn_with(traced, thread, &prompt, key, choice)
                        .await,
                );
            }
            Request::TurnStart {
                thread,
                prompt,
                background: true,
                budget,
                model,
                approval,
                meta: _,
            } => {
                let work = Work::Turn(prompt, Choice { model, approval });
                self.start_task(caller, thread, work, budget, None)
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::TurnCancel { thread } => self.cancel(thread).await,
            Request::ApprovalDecide {
                thread,
                approval,
                decision,
            } => self
                .inner
                .approvals
                .decide(thread, &approval, decision, caller.person)
                .map(|()| Reply::Done {}),
            Request::QuestionAnswer {
                thread,
                question,
                answers,
            } => self
                .inner
                .approvals
                .respond(thread, &question, answers)
                .map(|()| Reply::Done {}),
            Request::GoalStart {
                thread,
                objective,
                budget,
                background: false,
            } => {
                let goal = goal(objective, budget);
                return streamed(self.start_goal_as(traced, thread, goal).await);
            }
            Request::GoalStart {
                thread,
                objective,
                budget,
                background: true,
            } => {
                let goal = goal(objective, budget);
                let budget = goal.as_ref().map(|goal| goal.token_budget);
                self.start_task(caller, thread, Work::Goal(goal), budget, None)
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::GoalStop { thread } => self.stop_goal(caller, thread).await,
            Request::GoalGet { thread } => self.goal(thread).await.map(|goal| Reply::Goal { goal }),
            Request::TasksList { limit } => self.list_tasks(limit).await,
            Request::TasksStop { thread } => self.stop_task(caller, thread).await,
            Request::ScheduleCreate {
                when,
                prompt,
                goal,
                budget,
                worktree,
            } => {
                let new = NewSchedule {
                    when,
                    prompt,
                    goal,
                    budget,
                    worktree,
                };
                self.create_schedule(caller, new).await
            }
            Request::ScheduleList {} => self.list_schedules().await,
            Request::ScheduleDelete { schedule } => self.delete_schedule(caller, schedule).await,
            Request::SpaceList { limit } => self.list_spaces(caller, limit).await,
            Request::SpaceGet { space } => self.get_space(caller, space).await,
            Request::SpaceCreate {
                name,
                members,
                folder,
            } => self.create_space(caller, name, members, folder).await,
            Request::SpaceRename { space, name } => self.rename_space(caller, space, name).await,
            Request::FolderList { path } => self.list_folders(caller, path).await,
            Request::MemoryList { scope } => self.remember(caller, Control::List { scope }).await,
            Request::MemorySave { text, scope } => {
                self.remember(caller, Control::Save { text, scope }).await
            }
            Request::MemoryEdit { memory, text } => {
                self.remember(caller, Control::Edit { memory, text }).await
            }
            Request::MemoryMove { memory, scope } => {
                self.remember(caller, Control::Move { memory, scope }).await
            }
            Request::MemoryForget { memory, all, scope } => {
                match crate::memory::forget(memory, all, scope) {
                    Ok(control) => self.remember(caller, control).await,
                    Err(refused) => Err(refused),
                }
            }
            Request::MemoryRestore { memory } => {
                self.remember(caller, Control::Restore { memory }).await
            }
            Request::ProviderConnect { kind, key, url } => {
                let new = NewProvider { kind, key, url };
                self.connect_provider(caller, new).await
            }
            Request::ProviderList {} => self.list_providers(caller).await,
            Request::ProviderRemove { provider } => self.remove_provider(caller, provider).await,
            Request::CredentialConnect {
                url,
                token,
                username,
            } => {
                let new = NewCredential {
                    url,
                    token,
                    username,
                };
                self.connect_credential(caller, new).await
            }
            Request::CredentialList {} => self.list_credentials(caller).await,
            Request::CredentialRemove { credential } => {
                self.remove_credential(caller, credential).await
            }
            Request::CredentialDeviceStart {} => self.start_device(caller).await,
            Request::CredentialDevicePoll { flow } => self.poll_device(caller, flow).await,
        })
    }
}

/// The goal `goal/start` sets: none to continue the thread's own.
fn goal(objective: Option<String>, budget: Option<u64>) -> Option<Goal> {
    objective.map(|objective| Goal {
        objective,
        token_budget: budget.unwrap_or(DEFAULT_BUDGET),
    })
}

/// A turn's budget is enforced only for a turn that runs as a task: refused anywhere else
/// rather than ignored.
fn unenforced_budget() -> ApiError {
    ApiError::new(
        code::INVALID_PARAMS,
        "turn/start takes a budget only with background: true; for a turn you follow, use a goal",
    )
}
