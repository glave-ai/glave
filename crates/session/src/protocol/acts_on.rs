//! What a request acts on, which decides who may make it: the thread whose project the caller
//! must own, or the project whose memory it reads or writes.

use glave_event::ThreadId;
use glave_ports::ProjectId;

use super::Request;

impl Request {
    /// The thread a request acts on, when it names one: whose project decides whether the
    /// caller may make it.
    pub fn thread(&self) -> Option<ThreadId> {
        match self {
            Self::ThreadArchive { thread }
            | Self::ThreadUnarchive { thread }
            | Self::ThreadDelete { thread }
            | Self::ThreadRename { thread, .. }
            | Self::ThreadGet { thread }
            | Self::ThreadEvents { thread, .. }
            | Self::ThreadSources { thread }
            | Self::ThreadChanges { thread }
            | Self::ThreadSteps { thread }
            | Self::ThreadPlan { thread }
            | Self::PlanEdit { thread, .. }
            | Self::PlanBuild { thread }
            | Self::ThreadWatch { thread, .. }
            | Self::ThreadUnwatch { thread }
            | Self::TurnStart { thread, .. }
            | Self::TurnCancel { thread }
            | Self::ThreadCompact { thread }
            | Self::ApprovalDecide { thread, .. }
            | Self::QuestionAnswer { thread, .. }
            | Self::ServiceAnswer { thread, .. }
            | Self::TrustAnswer { thread, .. }
            | Self::GoalStart { thread, .. }
            | Self::GoalStop { thread }
            | Self::GoalGet { thread }
            | Self::TasksStop { thread }
            | Self::CommandList { thread }
            | Self::CommandStop { thread, .. } => Some(*thread),
            _ => None,
        }
    }

    /// For a request on a project's memory, the project it names; with none, it acts in the
    /// caller's Chats.
    pub(crate) fn memory_project(&self) -> Option<ProjectId> {
        match self {
            Self::MemoryList { project, .. }
            | Self::MemorySave { project, .. }
            | Self::MemoryEdit { project, .. }
            | Self::MemoryMove { project, .. }
            | Self::MemoryForget { project, .. }
            | Self::MemoryRestore { project, .. } => *project,
            _ => None,
        }
    }
}
