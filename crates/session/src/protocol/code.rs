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
    NOT_ALLOWED = -32012, "The caller may not do this";
    NO_SIGN_IN = -32013, "No valid sign-in (network transports)";
    ARCHIVED = -32014, "The thread is archived; unarchive it first";
    LIMIT = -32015, "A policy limit was reached";
    NO_SCHEDULE = -32016, "No such schedule";
    NO_MEMORY = -32017, "No such memory here";
    MEMORY_OFF = -32018, "Memory is off for this server";
    NO_SPACE = -32019, "No such space";
    PROVIDER = -32020, "The model provider failed";
    NO_MODEL = -32021, "No such model here, or not one this server may run";
    NOT_AVAILABLE = -32022, "Not available yet, such as a thread on a computer of the person's own";
    NO_PROVIDER = -32023, "No such model account here";
    NO_CREDENTIAL = -32024, "No such git host, or sign-in, here";
    HOST = -32025, "The git host refused, or could not be reached";
    NO_TRUST = -32026, "No such ask waits: the project changed since, or asks nothing now";
    QUEUE_FULL = -32027, "As many messages wait for the running turn as it takes";
}
