//! The one dispatch: every request, from any transport, is answered by [`Server::call`],
//! either at once with a reply or with a turn to follow. A transport carries the answer; it
//! never decides what a method does.

use std::pin::Pin;

use glave_harness::Change;
use glave_ports::memories::Control;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::accounts::KeptConnector;
use crate::choice::Choice;
use crate::connectors::Narrowed;
use crate::credentials::NewCredential;
use crate::lists::ListWatch;
use crate::protocol::{ApiError, LinkKind, Reply, Request, ServerInfo, code};
use crate::providers::NewProvider;
use crate::schedule::NewSchedule;
use crate::server::Server;
use crate::signups::Emailed;
use crate::steer::{Message, Sent};
use crate::tasks::Work;
use crate::turn::TurnStream;
use crate::watch::Watch;
use crate::words::Sentence;

/// What a request is answered with.
pub enum Answered {
    /// The reply, or why there is none.
    Reply(Result<Reply, ApiError>),
    /// A turn that someone follows: `turn/start` without `background`, when it starts one.
    /// Its notifications, then how it ended.
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
            let org = self.org_of(caller);
            self.inner.hub.setting_changed(org, caller.person, setting);
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

    /// Starts `request`, a turn someone follows, and streams it; a message sent
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
                    format!("{method} answered at once: only a turn it follows streams"),
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
                boxed(|| self.reread()).await;
                let organization = match boxed(|| self.organization_shown(caller)).await {
                    Ok(shown) => shown,
                    Err(failed) => return Answered::Reply(Err(failed)),
                };
                let invites = match boxed(|| self.may_invite(caller)).await {
                    Ok(may) => may,
                    Err(failed) => return Answered::Reply(Err(failed)),
                };
                let role = if organization && caller.auth.is_some() {
                    match boxed(|| self.role_of(caller)).await {
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
                    permissions: self
                        .within_sandbox(caller.person, self.inner.harness.defaults().permissions),
                    modes: self.modes(caller.person),
                    unsandboxed: self
                        .unsandboxed(caller.person)
                        .map(crate::protocol::Told::from),
                    id: self.server_id(),
                    origin: self.inner.origin.as_ref().map(ToString::to_string),
                    here: self.passkeys_here(),
                    signin: self.signin_ways(),
                    exists: crate::protocol::Exists {
                        organization,
                        credits: self.inner.credits.is_some(),
                    },
                    role,
                    email: self.inner.mail.is_some() && self.inner.origin.is_some(),
                    invites,
                    push: self.inner.push.is_some(),
                }))
            }
            Request::PeopleList { after, limit } => {
                boxed(|| self.list_people(caller, after, limit)).await
            }
            Request::RoleSet { person, role } => {
                boxed(|| self.set_role(caller, person, role)).await
            }
            Request::PersonRemove { person } => boxed(|| self.remove_person(caller, person)).await,
            Request::PersonName { name } => boxed(|| self.name_person(caller, &name)).await,
            Request::InviteCreate { role, email } => {
                boxed(|| self.create_invite(caller, role, email.as_deref())).await
            }
            Request::InviteList {} => boxed(|| self.list_invites(caller)).await,
            Request::InviteRevoke { invite } => boxed(|| self.revoke_invite(caller, invite)).await,
            Request::OrganizationList {} => boxed(|| self.list_organizations(caller)).await,
            Request::CreditsGet {} => boxed(|| self.get_credits(caller)).await,
            Request::CreditsAdd {
                amount,
                reference,
                organization,
            } => boxed(|| self.add_credits(caller, amount, &reference, organization)).await,
            Request::CreditsLimit { limit } => {
                boxed(|| self.set_credits_limit(caller, limit)).await
            }
            Request::InviteShow { code } => boxed(|| self.show_invite(caller, code.expose())).await,
            Request::InviteAccept { code } => {
                boxed(|| self.accept_invite(caller, code.expose())).await
            }
            Request::AwakeGet {} => Ok(boxed(|| self.awake()).await),
            Request::AwakeSet { on } => boxed(|| self.set_awake(caller, on)).await,
            Request::PushGet { endpoint } => {
                boxed(|| self.push_info(caller, endpoint.as_deref())).await
            }
            Request::PushSubscribe { browser } => boxed(|| self.subscribe(caller, browser)).await,
            Request::PushRemove { device } => boxed(|| self.remove_device(caller, device)).await,
            Request::LanguageGet {} => boxed(|| self.language(caller)).await,
            Request::LanguageSet { language } => {
                boxed(|| self.set_language(caller, language)).await
            }
            Request::GroupingGet {} => boxed(|| self.grouping(caller)).await,
            Request::GroupingSet { grouping } => {
                boxed(|| self.set_grouping(caller, grouping)).await
            }
            Request::FiltersGet {} => boxed(|| self.filters(caller)).await,
            Request::FiltersSet { filters } => boxed(|| self.set_filters(caller, filters)).await,
            Request::AppearanceGet {} => boxed(|| self.appearance(caller)).await,
            Request::AppearanceSet { accent } => {
                boxed(|| self.set_appearance(caller, accent)).await
            }
            Request::AppearancePreview { color, adjust } => Ok(Reply::Shades {
                shades: crate::accent::shades(color, adjust),
            }),
            Request::BrandGet {} => boxed(|| self.brand(caller)).await,
            Request::BrandSet {
                name,
                logo,
                accent,
                locked,
            } => boxed(|| self.set_brand(caller, name, logo, accent, locked)).await,
            Request::BrandLogoUpload { data } => boxed(|| self.upload_logo(caller, &data)).await,
            Request::BrandLogoGet {} => boxed(|| self.logo(caller)).await,
            Request::SigninLink {} => self.link(caller),
            Request::SigninSetup {} => self.setup(caller),
            Request::PasskeyCreateOptions { code, link } => match link {
                LinkKind::Setup => self.create_options(&code),
                LinkKind::Invite => boxed(|| self.join_options(&code)).await,
                LinkKind::Signup => boxed(|| self.link_options(&code, Emailed::Signup)).await,
                LinkKind::Recovery => boxed(|| self.link_options(&code, Emailed::Recovery)).await,
            },
            Request::SignupStart { email } => {
                boxed(|| self.start_link(caller, &email, Emailed::Signup)).await
            }
            Request::RecoveryStart { email } => {
                boxed(|| self.start_link(caller, &email, Emailed::Recovery)).await
            }
            Request::PasskeyCreate {
                ceremony,
                credential,
                name,
                person_name,
            } => boxed(|| self.create(&ceremony, &credential, name.as_deref(), person_name)).await,
            Request::PasskeyAddOptions {} => boxed(|| self.add_options(caller)).await,
            Request::PasskeyAdd {
                ceremony,
                credential,
                name,
            } => boxed(|| self.add(caller, &ceremony, &credential, name.as_deref())).await,
            Request::PasskeyGetOptions {} => boxed(|| self.get_options()).await,
            Request::PasskeyGet {
                ceremony,
                credential,
            } => boxed(|| self.get(&ceremony, &credential)).await,
            Request::PasskeyList {} => boxed(|| self.passkeys(caller)).await,
            Request::PasskeyRemove { passkey } => {
                boxed(|| self.remove_passkey(caller, &passkey)).await
            }
            Request::SigninRedeem { code } => boxed(|| self.redeem(code.expose())).await,
            Request::SigninEnd {} => boxed(|| self.end_session(caller)).await,
            Request::SigninEndOthers {} => boxed(|| self.end_others(caller)).await,
            Request::ModelList {} => {
                boxed(|| self.reread()).await;
                boxed(|| self.list_models(caller)).await
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
                boxed(|| self.start_thread(caller, project, worktree, background, &choice)).await
            }
            Request::ThreadList {
                project,
                limit,
                show,
                cursor,
            } => boxed(|| self.list_threads(caller, project, limit, show, cursor)).await,
            Request::ThreadGet { thread } => boxed(|| self.get_thread(caller, thread)).await,
            Request::ThreadFork { thread, at } => boxed(|| self.fork(caller, thread, at)).await,
            Request::ThreadRename { thread, title } => {
                boxed(|| self.rename(caller, thread, title)).await
            }
            Request::ThreadArchive { thread } => {
                boxed(|| self.change(caller, thread, Change::Archive)).await
            }
            Request::ThreadUnarchive { thread } => {
                boxed(|| self.change(caller, thread, Change::Unarchive)).await
            }
            Request::ThreadDelete { thread } => {
                boxed(|| self.change(caller, thread, Change::Delete)).await
            }
            Request::ThreadEvents {
                thread,
                after,
                limit,
            } => boxed(|| self.events(thread, after, limit)).await,
            Request::ThreadTurns { thread, before } => boxed(|| self.turns(thread, before)).await,
            Request::ThreadSources { thread } => boxed(|| self.sources(thread)).await,
            Request::ThreadChanges { thread } => boxed(|| self.changes(thread)).await,
            Request::ThreadSteps { thread } => boxed(|| self.steps(thread)).await,
            Request::ThreadPlan { thread } => boxed(|| self.plan(thread)).await,
            Request::PlanEdit { thread, steps } => {
                boxed(|| self.edit_plan(caller, thread, steps)).await
            }
            Request::PlanBuild { thread } => {
                return streamed(boxed(|| self.build_plan(traced, thread)).await);
            }
            Request::ThreadWatch { thread, after } => {
                return match boxed(|| self.watch(caller, thread, after)).await {
                    Ok(watch) => Answered::Watch(watch),
                    Err(refused) => Answered::Reply(Err(refused)),
                };
            }
            // A watch belongs to the connection it was asked on, which ends it (`rpc.rs`); in
            // the server there is nothing to end.
            Request::ThreadUnwatch { .. } => Ok(Reply::Done {}),
            Request::ThreadListWatch {
                project,
                after,
                away,
            } => {
                return match boxed(|| self.watch_list(caller, project, after, away)).await {
                    Ok(watch) => Answered::List(watch),
                    Err(refused) => Answered::Reply(Err(refused)),
                };
            }
            // Only for a project the caller owns, as watching it is.
            Request::ThreadListUnwatch { project } => match project {
                Some(project) => boxed(|| self.owned(caller, project))
                    .await
                    .map(|_| Reply::Done {}),
                None => Ok(Reply::Done {}),
            },
            Request::ThreadCompact { thread } => boxed(|| self.compact(thread)).await,
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
                let sent = boxed(|| self.start_turn_with(traced, thread, message, key)).await;
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
                let work = Work(prompt, choice);
                boxed(|| self.start_task(caller, thread, work, budget, None))
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::TurnCancel { thread } => boxed(|| self.cancel(thread)).await,
            Request::FileUpload {
                project,
                name,
                media_type,
                data,
            } => boxed(|| self.upload(caller, project, &name, &media_type, &data)).await,
            Request::ApprovalDecide {
                thread,
                approval,
                decision,
            } => boxed(|| self.decide(caller, thread, approval, decision)).await,
            Request::QuestionAnswer {
                thread,
                question,
                answers,
            } => boxed(|| self.answer_questions(caller, thread, question, answers)).await,
            Request::ServiceAnswer {
                thread,
                call,
                asked,
                answer,
            } => boxed(|| self.answer_service(caller, thread, call, asked, answer)).await,
            Request::TrustAnswer {
                thread,
                ask,
                answer,
            } => boxed(|| self.answer_trust(caller, thread, &ask, answer)).await,
            Request::TasksList { limit } => boxed(|| self.list_tasks(caller, limit)).await,
            Request::TasksStop { thread } => boxed(|| self.stop_task(caller, thread)).await,
            Request::TerminalOpen { thread, size } => {
                boxed(|| self.open_terminal(caller, thread, size)).await
            }
            Request::TerminalList { thread } => boxed(|| self.list_terminals(caller, thread)).await,
            Request::TerminalWrite {
                thread,
                terminal,
                input,
            } => boxed(|| self.write_terminal(caller, thread, (terminal, &input))).await,
            Request::TerminalRead {
                thread,
                terminal,
                from,
                wait_ms,
            } => {
                let wait = std::time::Duration::from_millis(wait_ms);
                boxed(|| self.read_terminal(caller, thread, (terminal, from), wait)).await
            }
            Request::TerminalResize {
                thread,
                terminal,
                size,
            } => boxed(|| self.resize_terminal(caller, thread, (terminal, size))).await,
            Request::TerminalClose { thread, terminal } => {
                boxed(|| self.close_terminal(caller, thread, terminal)).await
            }
            Request::CommandList { thread } => boxed(|| self.list_commands(caller, thread)).await,
            Request::CommandStop { thread, command } => {
                boxed(|| self.stop_command(caller, thread, command)).await
            }
            Request::ScheduleCreate {
                project,
                when,
                prompt,
                budget,
            } => {
                let new = NewSchedule {
                    project,
                    when,
                    prompt,
                    budget,
                };
                boxed(|| self.create_schedule(caller, new)).await
            }
            Request::ScheduleList {} => boxed(|| self.list_schedules(caller)).await,
            Request::ScheduleDelete { schedule } => {
                boxed(|| self.delete_schedule(caller, schedule)).await
            }
            Request::ProjectList { limit } => boxed(|| self.list_projects(caller, limit)).await,
            Request::ProjectGet { project } => boxed(|| self.get_project(caller, project)).await,
            Request::ProjectFind { path } => boxed(|| self.find_project(caller, &path)).await,
            Request::ProjectCreate { name, folder } => {
                boxed(|| self.create_project(caller, name, folder)).await
            }
            Request::ProjectRename { project, name } => {
                boxed(|| self.rename_project(caller, project, name)).await
            }
            Request::ProjectFolderSet { project, path } => {
                boxed(|| self.move_folder(caller, project, path)).await
            }
            Request::ProjectRemove { project } => {
                boxed(|| self.remove_project(caller, project)).await
            }
            Request::FolderList { path } => boxed(|| self.list_folders(caller, path)).await,
            Request::MemoryGet {} => boxed(|| self.remembering(caller)).await,
            Request::MemorySet { on } => boxed(|| self.set_remembering(caller, on)).await,
            Request::MemoryList { scope, project } => {
                boxed(|| self.remember(caller, project, Control::List { scope })).await
            }
            Request::MemorySave {
                text,
                scope,
                project,
            } => boxed(|| self.remember(caller, project, Control::Save { text, scope })).await,
            Request::MemoryEdit {
                memory,
                text,
                project,
            } => boxed(|| self.remember(caller, project, Control::Edit { memory, text })).await,
            Request::MemoryMove {
                memory,
                scope,
                project,
            } => boxed(|| self.remember(caller, project, Control::Move { memory, scope })).await,
            Request::MemoryForget {
                memory,
                all,
                scope,
                project,
            } => match crate::memory::forget(memory, all, scope) {
                Ok(control) => boxed(|| self.remember(caller, project, control)).await,
                Err(refused) => Err(refused),
            },
            Request::MemoryRestore { memory, project } => {
                boxed(|| self.remember(caller, project, Control::Restore { memory })).await
            }
            Request::ProviderConnect { kind, key, url } => {
                let new = NewProvider { kind, key, url };
                boxed(|| self.connect_provider(caller, new)).await
            }
            Request::ProviderList {} => boxed(|| self.list_providers(caller)).await,
            Request::ProviderRefresh {} => boxed(|| self.refresh_providers(caller)).await,
            Request::ProviderRemove { provider } => {
                boxed(|| self.remove_provider(caller, provider)).await
            }
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
                boxed(|| self.connect_credential(caller, new)).await
            }
            Request::CredentialList {} => boxed(|| self.list_credentials(caller)).await,
            Request::CredentialRemove { credential } => {
                boxed(|| self.remove_credential(caller, credential)).await
            }
            Request::CredentialDeviceStart {} => boxed(|| self.start_device(caller)).await,
            Request::CredentialDevicePoll { flow } => {
                boxed(|| self.poll_device(caller, flow)).await
            }
            Request::SecretAdd { name, value } => {
                boxed(|| self.add_secret(caller, &name, &value)).await
            }
            Request::SecretList {} => boxed(|| self.list_secrets(caller)).await,
            Request::SecretReplace { name, value } => {
                boxed(|| self.replace_secret(caller, &name, &value)).await
            }
            Request::SecretRemove { name } => boxed(|| self.remove_secret(caller, &name)).await,
            Request::ConnectionStart { server } => {
                boxed(|| self.start_connection(caller, server)).await
            }
            Request::ConnectionPoll { flow } => boxed(|| self.poll_connection(caller, flow)).await,
            Request::ConnectionList {} => boxed(|| self.list_connections(caller)).await,
            Request::SearchGet { usage } => boxed(|| self.get_search(caller, usage)).await,
            Request::SearchSet { on } => boxed(|| self.set_search(caller, on)).await,
            Request::ConnectionRemove { server } => {
                boxed(|| self.remove_connection(caller, server)).await
            }
            Request::ConnectorList {
                company,
                category,
                tag,
                used_in,
            } => {
                let narrowed = Narrowed {
                    company,
                    category,
                    tag,
                };
                boxed(|| self.list_connectors(caller, &narrowed, used_in)).await
            }
            Request::ConnectorConnect {
                connector,
                account,
                only,
                another,
            } => {
                let connector = KeptConnector {
                    id: connector,
                    account,
                    only,
                };
                boxed(|| self.connect_connector(caller, connector, another)).await
            }
            Request::ConnectorPlace {
                connector,
                account,
                only,
            } => {
                let connector = KeptConnector {
                    id: connector,
                    account,
                    only,
                };
                boxed(|| self.place_connector(caller, connector)).await
            }
            Request::ConnectorDisconnect { connector, account } => {
                let connector = KeptConnector {
                    id: connector,
                    account,
                    only: None,
                };
                boxed(|| self.disconnect_connector(caller, connector)).await
            }
            Request::ConnectorRename {
                connector,
                account,
                name,
            } => {
                let connector = KeptConnector {
                    id: connector,
                    account,
                    only: None,
                };
                boxed(|| self.rename_connector(caller, connector, name)).await
            }
        })
    }
}

/// A handler's future, built here and on the heap. Unoptimized, a function keeps a stack slot for
/// every temporary of every arm at once, so awaiting each handler in `Server::answer` itself
/// gave it a frame over 1 MiB, the sum of all of them, and a test thread has 2 (#208): a change
/// to any handler could overflow it, on Postgres first, whose futures are the largest. Only the
/// closure and the box stay in that frame.
fn boxed<F: Future>(build: impl FnOnce() -> F) -> Pin<Box<F>> {
    Box::pin(build())
}

/// A turn's budget is enforced only for a turn that runs as a task: refused anywhere else
/// rather than ignored.
fn unenforced_budget() -> ApiError {
    ApiError::new(code::INVALID_PARAMS, Sentence::BudgetOnlyInBackground)
}
