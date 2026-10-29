//! The one dispatch: every request, from any transport, is answered by [`Server::call`],
//! either at once with a reply or with a turn to follow. A transport carries the answer; it
//! never decides what a method does.

use glave_harness::{Change, DEFAULT_BUDGET, Goal};
use glave_ports::memories::Control;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::choice::Choice;
use crate::credentials::NewCredential;
use crate::lists::ListWatch;
use crate::protocol::{ApiError, Reply, Request, ServerInfo, code};
use crate::providers::NewProvider;
use crate::schedule::NewSchedule;
use crate::server::Server;
use crate::steer::{Message, Sent};
use crate::tasks::Work;
use crate::turn::TurnStream;
use crate::watch::Watch;
use crate::words::Sentence;

/// What a request is answered with.
pub enum Answered {
    /// The reply, or why there is none.
    Reply(Result<Reply, ApiError>),
    /// A turn, or a goal's turns, that someone follows: `turn/start` and `goal/start` without
    /// `background`, when they start one. Its notifications, then how it ended.
    Stream(TurnStream),
    /// A thread watched (`thread/watch`): its notifications, until it ends.
    Watch(Watch),
    /// A list watched (`thread/list/watch`): its threads' summaries as they change, until it
    /// is dropped.
    List(ListWatch),
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `request` from `caller`, the person the transport verified. A request on a
    /// thread, or on the server's memory, is refused with `-32012` unless the caller owns that
    /// project; tasks and schedules are those of the projects they own.
    pub async fn call(&self, caller: &Caller, request: Request) -> Answered {
        let span = crate::trace::span(&request);
        let changes = request.changes();
        let answered = tracing::Instrument::instrument(self.answer(caller, request), span).await;
        // A sign-in still waiting for the person has changed nothing yet.
        if let (Some(setting), Answered::Reply(Ok(reply))) = (changes, &answered)
            && !matches!(reply, Reply::Waiting { .. })
        {
            self.inner.hub.setting_changed(caller.person, setting);
        }
        answered
    }

    /// Answers one request from `caller`; a turn, or a watch, is answered once it has ended.
    pub async fn request_as(&self, caller: &Caller, request: Request) -> Result<Reply, ApiError> {
        match self.call(caller, request).await {
            Answered::Reply(reply) => reply,
            Answered::Stream(mut stream) => stream.finish().await.map(|turn| Reply::Turn { turn }),
            Answered::Watch(mut watch) => {
                watch.finish().await.map(|ended| Reply::Watched { ended })
            }
            Answered::List(mut watch) => watch.finish().await.map(|ended| Reply::Watched { ended }),
        }
    }

    /// Answers one request from the local person; see [`Server::request_as`].
    pub async fn request(&self, request: Request) -> Result<Reply, ApiError> {
        self.request_as(&Caller::default(), request).await
    }

    /// Starts `request`, a turn or a goal someone follows, and streams it; a message sent
    /// while a turn runs in its thread is queued to that turn instead, which this does not
    /// follow. Any other request is answered at once, so it is refused here.
    pub async fn start_streaming(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<Sent<TurnStream>, ApiError> {
        let method = request.method();
        match self.call(caller, request).await {
            Answered::Stream(stream) => Ok(Sent::Started(stream)),
            Answered::Reply(Ok(Reply::Queued { queued })) => Ok(Sent::Queued(queued)),
            Answered::Watch(_) | Answered::List(_) => Err(ApiError::new(
                code::INVALID_PARAMS,
                format!("{method} is followed with Server::watch or Server::watch_list"),
            )),
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
        if let Err(refused) = self.seen(caller, &request).await {
            return Answered::Reply(Err(refused));
        }
        // The thread a request names keeps its place until it is answered.
        let _held = match self.allow(caller, &request).await {
            Ok(held) => held,
            Err(refused) => return Answered::Reply(Err(refused)),
        };
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
            Request::Discover {} => {
                self.reread().await;
                let organization = match self.organization_shown(caller).await {
                    Ok(shown) => shown,
                    Err(failed) => return Answered::Reply(Err(failed)),
                };
                let role = if organization && caller.auth.is_some() {
                    match self.role_of(caller).await {
                        Ok(role) => Some(role),
                        Err(failed) => return Answered::Reply(Err(failed)),
                    }
                } else {
                    None
                };
                Ok(Reply::Server(ServerInfo {
                    name: "glave".to_owned(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    protocols: vec![crate::protocol::PROTOCOL.to_owned()],
                    permissions: self.inner.harness.defaults().permissions,
                    modes: self.modes(),
                    unsandboxed: self.unsandboxed().map(crate::protocol::Told::from),
                    id: self.server_id(),
                    origin: self.inner.origin.as_ref().map(ToString::to_string),
                    here: self.passkeys_here(),
                    signin: self.signin_ways(),
                    exists: crate::protocol::Exists {
                        organization,
                        workers: self.inner.worker.is_some(),
                    },
                    role,
                    email: self.inner.mail.is_some() && self.inner.origin.is_some(),
                }))
            }
            Request::PeopleList { after, limit } => self.list_people(caller, after, limit).await,
            Request::RoleSet { person, role } => self.set_role(caller, person, role).await,
            Request::PersonRemove { person } => self.remove_person(caller, person).await,
            Request::InviteCreate { role, email } => {
                self.create_invite(caller, role, email.as_deref()).await
            }
            Request::InviteList {} => self.list_invites(caller).await,
            Request::InviteRevoke { invite } => self.revoke_invite(caller, invite).await,
            Request::InviteShow { code } => self.show_invite(code.expose()).await,
            Request::InviteAccept { code } => self.accept_invite(caller, code.expose()).await,
            Request::AwakeGet {} => Ok(self.awake().await),
            Request::AwakeSet { on } => self.set_awake(caller, on).await,
            Request::LanguageGet {} => self.language(caller).await,
            Request::LanguageSet { language } => self.set_language(caller, language).await,
            Request::GroupingGet {} => self.grouping(caller).await,
            Request::GroupingSet { grouping } => self.set_grouping(caller, grouping).await,
            Request::SigninLink {} => self.link(caller),
            Request::SigninSetup {} => self.setup(caller),
            Request::PasskeyCreateOptions { code, invite } => {
                if invite {
                    self.join_options(&code).await
                } else {
                    self.create_options(&code)
                }
            }
            Request::PasskeyCreate {
                ceremony,
                credential,
                name,
            } => self.create(&ceremony, &credential, name.as_deref()).await,
            Request::PasskeyAddOptions {} => self.add_options(caller).await,
            Request::PasskeyAdd {
                ceremony,
                credential,
                name,
            } => {
                self.add(caller, &ceremony, &credential, name.as_deref())
                    .await
            }
            Request::PasskeyGetOptions {} => self.get_options().await,
            Request::PasskeyGet {
                ceremony,
                credential,
            } => self.get(&ceremony, &credential).await,
            Request::PasskeyList {} => self.passkeys(caller).await,
            Request::PasskeyRemove { passkey } => self.remove_passkey(caller, &passkey).await,
            Request::SigninRedeem { code } => self.redeem(code.expose()).await,
            Request::SigninEnd {} => self.end_session(caller).await,
            Request::SigninEndOthers {} => self.end_others(caller).await,
            Request::ModelList {} => {
                self.reread().await;
                Ok(self.list_models())
            }
            Request::ThreadStart {
                computer: Some(computer),
                ..
            } => Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::NoComputerYet {
                    computer: computer.to_string(),
                },
            )),
            Request::ThreadStart {
                project,
                worktree,
                background,
                computer: None,
                model,
                permissions,
                effort,
            } => {
                let choice = Choice {
                    model,
                    permissions,
                    effort,
                };
                self.start_thread(caller, project, worktree, background, &choice)
                    .await
            }
            Request::ThreadList {
                project,
                limit,
                show,
                cursor,
            } => {
                self.list_threads(caller, project, limit, show, cursor)
                    .await
            }
            Request::ThreadGet { thread } => self.get_thread(caller, thread).await,
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
            Request::ThreadSources { thread } => self.sources(thread).await,
            Request::ThreadChanges { thread } => self.changes(thread).await,
            Request::ThreadSteps { thread } => self.steps(thread).await,
            Request::ThreadPlan { thread } => self.plan(thread).await,
            Request::PlanEdit { thread, steps } => self.edit_plan(caller, thread, steps).await,
            Request::PlanBuild { thread } => {
                return streamed(self.build_plan(traced, thread).await);
            }
            Request::ThreadWatch { thread, after } => {
                return match self.watch(caller, thread, after).await {
                    Ok(watch) => Answered::Watch(watch),
                    Err(refused) => Answered::Reply(Err(refused)),
                };
            }
            // A watch belongs to the connection it was asked on, which ends it (`rpc.rs`); in
            // the server there is nothing to end.
            Request::ThreadUnwatch { .. } => Ok(Reply::Done {}),
            Request::ThreadListWatch { project, after } => {
                return match self.watch_list(caller, project, after).await {
                    Ok(watch) => Answered::List(watch),
                    Err(refused) => Answered::Reply(Err(refused)),
                };
            }
            // Only for a project the caller owns, as watching it is.
            Request::ThreadListUnwatch { project } => match project {
                Some(project) => self.owned(caller, project).await.map(|_| Reply::Done {}),
                None => Ok(Reply::Done {}),
            },
            Request::ThreadCompact { thread } => self.compact(thread).await,
            Request::TurnStart {
                thread,
                prompt,
                files,
                background: false,
                budget,
                model,
                permissions,
                effort,
                unattended,
                meta,
            } => {
                if budget.is_some() {
                    return Answered::Reply(Err(unenforced_budget()));
                }
                let key = meta.idempotency_key.as_ref();
                let message = Message {
                    prompt: &prompt,
                    files: &files,
                    choice: Choice {
                        model,
                        permissions,
                        effort,
                    },
                    queue: true,
                    unattended,
                };
                let sent = self.start_turn_with(traced, thread, message, key).await;
                return match sent {
                    Ok(Sent::Started(stream)) => Answered::Stream(stream),
                    Ok(Sent::Queued(queued)) => Answered::Reply(Ok(Reply::Queued { queued })),
                    Err(refused) => Answered::Reply(Err(refused)),
                };
            }
            Request::TurnStart {
                thread,
                prompt,
                files,
                background: true,
                budget,
                model,
                permissions,
                effort,
                // A task is answered by no one anyway.
                unattended: _,
                meta: _,
            } => {
                if !files.is_empty() {
                    return Answered::Reply(Err(ApiError::new(
                        code::INVALID_PARAMS,
                        Sentence::FilesNotForTasks,
                    )));
                }
                let choice = Choice {
                    model,
                    permissions,
                    effort,
                };
                let work = Work::Turn(prompt, choice);
                self.start_task(caller, thread, work, budget, None)
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::TurnCancel { thread } => self.cancel(thread).await,
            Request::FileUpload {
                project,
                name,
                media_type,
                data,
            } => {
                self.upload(caller, project, &name, &media_type, &data)
                    .await
            }
            Request::ApprovalDecide {
                thread,
                approval,
                decision,
            } => self.decide(caller, thread, approval, decision).await,
            Request::QuestionAnswer {
                thread,
                question,
                answers,
            } => {
                self.answer_questions(caller, thread, question, answers)
                    .await
            }
            Request::TrustAnswer {
                thread,
                ask,
                answer,
            } => self.answer_trust(caller, thread, &ask, answer).await,
            Request::GoalStart {
                thread,
                objective,
                budget,
                background: false,
                unattended,
            } => {
                let goal = goal(objective, budget);
                let started = self.start_goal_as(traced, thread, goal, unattended);
                return streamed(started.await);
            }
            Request::GoalStart {
                thread,
                objective,
                budget,
                background: true,
                unattended: _,
            } => {
                let goal = goal(objective, budget);
                let budget = goal.as_ref().map(|goal| goal.token_budget);
                self.start_task(caller, thread, Work::Goal(goal), budget, None)
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::GoalStop { thread } => self.stop_goal(caller, thread).await,
            Request::GoalGet { thread } => self.goal(thread).await.map(|goal| Reply::Goal { goal }),
            Request::TasksList { limit } => self.list_tasks(caller, limit).await,
            Request::TasksStop { thread } => self.stop_task(caller, thread).await,
            Request::CommandList { thread } => self.list_commands(thread).await,
            Request::CommandStop { thread, command } => {
                self.stop_command(caller, thread, command).await
            }
            Request::ScheduleCreate {
                project,
                when,
                prompt,
                goal,
                budget,
            } => {
                let new = NewSchedule {
                    project,
                    when,
                    prompt,
                    goal,
                    budget,
                };
                self.create_schedule(caller, new).await
            }
            Request::ScheduleList {} => self.list_schedules(caller).await,
            Request::ScheduleDelete { schedule } => self.delete_schedule(caller, schedule).await,
            Request::ProjectList { limit } => self.list_projects(caller, limit).await,
            Request::ProjectGet { project } => self.get_project(caller, project).await,
            Request::ProjectFind { path } => self.find_project(caller, &path).await,
            Request::ProjectCreate { name, folder } => {
                self.create_project(caller, name, folder).await
            }
            Request::ProjectRename { project, name } => {
                self.rename_project(caller, project, name).await
            }
            Request::ProjectFolderSet { project, path } => {
                self.move_folder(caller, project, path).await
            }
            Request::ProjectRemove { project } => self.remove_project(caller, project).await,
            Request::FolderList { path } => self.list_folders(caller, path).await,
            Request::MemoryGet {} => self.remembering(caller).await,
            Request::MemorySet { on } => self.set_remembering(caller, on).await,
            Request::MemoryList { scope, project } => {
                self.remember(caller, project, Control::List { scope })
                    .await
            }
            Request::MemorySave {
                text,
                scope,
                project,
            } => {
                self.remember(caller, project, Control::Save { text, scope })
                    .await
            }
            Request::MemoryEdit {
                memory,
                text,
                project,
            } => {
                self.remember(caller, project, Control::Edit { memory, text })
                    .await
            }
            Request::MemoryMove {
                memory,
                scope,
                project,
            } => {
                self.remember(caller, project, Control::Move { memory, scope })
                    .await
            }
            Request::MemoryForget {
                memory,
                all,
                scope,
                project,
            } => match crate::memory::forget(memory, all, scope) {
                Ok(control) => self.remember(caller, project, control).await,
                Err(refused) => Err(refused),
            },
            Request::MemoryRestore { memory, project } => {
                self.remember(caller, project, Control::Restore { memory })
                    .await
            }
            Request::ProviderConnect { kind, key, url } => {
                let new = NewProvider { kind, key, url };
                self.connect_provider(caller, new).await
            }
            Request::ProviderList {} => self.list_providers(caller).await,
            Request::ProviderRefresh {} => self.refresh_providers(caller).await,
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
            Request::ConnectionStart { server } => self.start_connection(caller, server).await,
            Request::ConnectionPoll { flow } => self.poll_connection(caller, flow).await,
            Request::ConnectionList {} => self.list_connections(caller).await,
            Request::SearchGet { usage } => self.get_search(caller, usage).await,
            Request::SearchSet { on } => self.set_search(caller, on).await,
            Request::ConnectionRemove { server } => self.remove_connection(caller, server).await,
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
    ApiError::new(code::INVALID_PARAMS, Sentence::BudgetOnlyInBackground)
}
