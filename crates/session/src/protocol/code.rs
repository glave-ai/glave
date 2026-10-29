//! Every error a request can be answered with (`specs/session.md`): JSON-RPC's own, then
//! glave's. Each is a constant and a row of [`ALL`], from one table.

macro_rules! codes {
    ($($name:ident = $code:literal, $meaning:literal;)*) => {
        $(
            #[doc = $meaning]
            pub const $name: i64 = $code;
        )*

        /// Every code: its number, a name for clients, and its meaning.
        pub const ALL: &[(i64, &str, &str)] = &[$(($code, stringify!($name), $meaning)),*];
    };
}

codes! {
    PARSE = -32700, "The message is not JSON-RPC";
    INVALID_REQUEST = -32600, "The message is not a valid request";
    NO_METHOD = -32601, "No such method";
    INVALID_PARAMS = -32602, "The params are not valid";
    INTERNAL = -32603, "The server failed";
    NO_THREAD = -32001, "No such thread";
    TURN_RUNNING = -32002, "A turn is already running in this thread";
    NO_TURN = -32003, "No turn is running in this thread";
    NO_APPROVAL = -32004, "No such approval is pending";
    CANCELLED = -32005, "The operation was cancelled";
    NO_QUESTION = -32006, "No such question is waiting";
    NO_GOAL = -32007, "The thread has no active goal (or, to resume, no paused one)";
    PROMPT_BLOCKED = -32008, "A hook stopped the prompt";
    WORKSPACE = -32009, "The thread's workspace could not be opened";
    RUNNING_ELSEWHERE = -32011, "The thread is working on another server, which holds it";
    NOT_ALLOWED = -32012, "The caller may not do this";
    NO_SIGN_IN = -32013, "No valid sign-in (network transports)";
    ARCHIVED = -32014, "The thread is archived; unarchive it first";
    LIMIT = -32015, "A policy limit was reached";
    NO_SCHEDULE = -32016, "No such schedule";
    NO_MEMORY = -32017, "No such memory here";
    MEMORY_OFF = -32018, "Memory is off for this server";
    NO_PROJECT = -32019, "No such project";
    PROVIDER = -32020, "The model provider failed";
    NO_MODEL = -32021, "No such model here, or not one this server may run";
    NOT_AVAILABLE = -32022, "Not available yet, such as a thread on a computer of the person's own";
    NO_PROVIDER = -32023, "No such model account here";
    NO_CREDENTIAL = -32024, "No such git host, connection or sign-in here";
    HOST = -32025, "The git host, or a connection's server or its sign-in, refused or could not be reached";
    NO_TRUST = -32026, "No such ask waits: the project changed since, or asks nothing now";
    NO_COMMAND = -32027, "No such command is running in the background in this thread";
    QUEUE_FULL = -32028, "As many messages wait for the running turn as it takes";
    NO_FILE = -32030, "No such file here: never kept in this project, or no message used it within a day";
    NO_IMAGES = -32031, "The thread's model does not read images";
    NO_SERVER = -32032, "No remote server of the person's settings goes by that name or address";
    NO_PLAN = -32033, "The thread has no plan waiting for the person's approval";
    NO_PERSON = -32034, "No one by that id has signed in to this organization";
    NO_INVITE = -32035, "No such invite waits here: it was used, revoked or has expired, or it is another organization's";
    NO_FOLDER = -32029, "The project's folder isn't there anymore: give it its new place (`project/folder/set`) or remove the project (`project/remove`)";
}
