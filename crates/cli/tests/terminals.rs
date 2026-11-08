//! A person's terminals, opened by glave's own program, which starts their shell in a session
//! of its own: this machine's computers pass the terminals suite with the real binary as their
//! helper, started directly and through the lifeline's gate, and so does
//! `glave computer serve --stdio`, across the protocol. A person's computer as a server holds
//! it finds them as it is awake, never woken to say so. The person's server opens theirs in a
//! thread's folder over the session API, and never one for someone else on its computer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use glave_computer::{LocalComputers, Peek, Wake, Waking};
use glave_ports::computer::{
    Bytes, ComputerError, ComputerId, Computers, Environment, Home, Screenful, Sources, TerminalId,
    TerminalSize,
};
use glave_ports::testing::{TempDir, read_program};
use glave_ports::{block_on, conformance};
use glave_process::Processes;
use glave_provider_openai::fake::FakeServer;
use glave_session::client::Remote;
use serde_json::{Value, json};

use crate::computer::served_on_stdio;
use crate::connected::request;
use crate::http::serve_http_with;
use crate::invited::invited;
use crate::support::*;

const GLAVE: &str = env!("CARGO_BIN_EXE_glave");

/// What a terminal's shell starts with: bash, at home in `home`, so the person's own profile
/// plays no part.
fn shell_in(home: &std::path::Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([
            ("HOME".into(), home.as_os_str().to_owned()),
            ("SHELL".into(), "/bin/bash".into()),
        ])
        .collect()
}

#[test]
fn this_machine_opens_terminals_with_glave_as_their_starter_directly_and_through_the_gate() {
    let dir = TempDir::new("terminals-here").unwrap();
    let folder = dir.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    read_program(GLAVE).unwrap();
    block_on!(async {
        let tied = Processes::unrecorded().with_lifeline(GLAVE).await.unwrap();
        for processes in [Processes::unrecorded(), tied] {
            let computers = LocalComputers::new(dir.join("copies"), shell_in(&dir))
                .with_helper(GLAVE)
                .with_processes(processes);
            let computer = computers.create(&Environment::host()).await.unwrap().id;
            let verdict =
                conformance::terminals(&computers, computer, &folder.to_string_lossy()).await;
            assert_eq!(verdict, Ok(()));
        }
    });
}

#[test]
fn the_binary_serves_terminals_across_the_protocol() {
    let dir = TempDir::new("terminals-served").unwrap();
    let folder = dir.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    read_program(GLAVE).unwrap();
    block_on!(async {
        let home = dir.join("home");
        let env = [
            ("GLAVE_HOME", home.as_os_str()),
            ("HOME", dir.as_os_str()),
            ("SHELL", "/bin/bash".as_ref()),
        ];
        let (mut child, remote) = served_on_stdio(&env).await;
        let computer = remote.create(&Environment::host()).await.unwrap().id;
        let verdict = conformance::terminals(&remote, computer, &folder.to_string_lossy()).await;
        assert_eq!(verdict, Ok(()));
        drop(remote);
        let status = child.wait().await.unwrap();
        assert!(status.success(), "the server ends cleanly with its input");
    });
}

const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };

/// How a person's computer stands, as [`Waking`]'s peek finds it.
const AWAKE: u8 = 0;
const ASLEEP: u8 = 1;
const ELSEWHERE: u8 = 2;

/// `home` as a server holds a person's computer: woken by `wakes` (each counted) and found as
/// `stands` says without waking it.
fn held(home: &Home, stands: &Arc<AtomicU8>, wakes: &Arc<AtomicUsize>) -> Home {
    let (found, woken) = (home.clone(), Arc::clone(wakes));
    let wake: Wake = Arc::new(move || {
        woken.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok(found.clone())))
    });
    let (found, stands) = (home.clone(), Arc::clone(stands));
    let peek: Peek = Arc::new(move || {
        let now = match stands.load(Ordering::SeqCst) {
            AWAKE => Ok(Some(found.clone())),
            ASLEEP => Ok(None),
            _ => Err(ComputerError::Unreachable("another server's".into())),
        };
        Box::pin(std::future::ready(now))
    });
    Waking::home((wake, peek), None, home.home.clone(), "/nowhere")
}

/// What `terminal` on `home`'s computer printed until it printed `marker`.
async fn printed(home: &Home, terminal: (TerminalId, &str), marker: &str) -> String {
    let (computer, mut seen, mut from) = (home.home.computer, Vec::new(), 0);
    for _ in 0..100 {
        let read =
            home.computers
                .read_terminal(computer, terminal, from, Duration::from_millis(300));
        let read = read.await.unwrap();
        seen.extend_from_slice(read.output.as_slice());
        from = read.next;
        if String::from_utf8_lossy(&seen).contains(marker) {
            return String::from_utf8_lossy(&seen).into_owned();
        }
    }
    panic!(
        "never printed {marker:?}: {}",
        String::from_utf8_lossy(&seen)
    );
}

#[test]
fn a_person_s_computer_finds_its_terminals_as_it_is_awake_and_never_wakes_to_say_so() {
    let dir = TempDir::new("terminals-waking").unwrap();
    let folder = dir.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    read_program(GLAVE).unwrap();
    block_on!(async {
        let computers = LocalComputers::new(dir.join("copies"), shell_in(&dir)).with_helper(GLAVE);
        let computers: Arc<dyn Computers> = Arc::new(computers);
        let computer: ComputerId = computers.create(&Environment::host()).await.unwrap().id;
        let path = folder.to_string_lossy().into_owned();
        let sources = Sources::Folder { path: path.clone() };
        let opened = computers.open(computer, &sources).await.unwrap();
        let home = Home {
            computers,
            home: opened,
        };
        let stands = Arc::new(AtomicU8::new(AWAKE));
        let (first_wakes, then_wakes) = (Arc::default(), Arc::default());
        let first = held(&home, &stands, &first_wakes);
        let owner = "org/project/thread";
        let id = first
            .computers
            .open_terminal(computer, &path, owner, SIZE)
            .await
            .unwrap();
        assert_eq!(
            first_wakes.load(Ordering::SeqCst),
            1,
            "opening one wakes it"
        );
        let terminal = (id, owner);

        // A server that never opened it, restarted or another, finds it where it is.
        let then = held(&home, &stands, &then_wakes);
        let listed = then.computers.terminals(computer, owner).await.unwrap();
        assert_eq!(listed.iter().map(|each| each.id).collect::<Vec<_>>(), [id]);
        let input = Bytes::new("echo found-$((40+2))\r");
        then.computers
            .write_terminal(computer, terminal, &input)
            .await
            .unwrap();
        printed(&then, terminal, "found-42").await;

        // Asleep, it has none; out of this server's reach, it says so, never that it has none.
        stands.store(ASLEEP, Ordering::SeqCst);
        assert_eq!(then.computers.terminals(computer, owner).await.unwrap(), []);
        let read = then
            .computers
            .read_terminal(computer, terminal, 0, Duration::ZERO);
        assert!(matches!(read.await, Err(ComputerError::NotFound(_))));
        stands.store(ELSEWHERE, Ordering::SeqCst);
        let listed = then.computers.terminals(computer, owner).await;
        assert!(
            matches!(listed, Err(ComputerError::Unreachable(_))),
            "{listed:?}"
        );
        let closed = then.computers.close_terminal(computer, terminal).await;
        assert!(
            matches!(closed, Err(ComputerError::Unreachable(_))),
            "{closed:?}"
        );
        assert_eq!(
            then_wakes.load(Ordering::SeqCst),
            0,
            "nothing but opening wakes it"
        );

        stands.store(AWAKE, Ordering::SeqCst);
        then.computers
            .close_terminal(computer, terminal)
            .await
            .unwrap();
        assert_eq!(
            first.computers.terminals(computer, owner).await.unwrap(),
            []
        );
        home.computers.destroy(computer).await.unwrap();
    });
}

/// What `caller`'s `method` with `params` answers, as JSON, or its error's.
async fn asked(caller: &Remote, method: &str, params: Value) -> Result<Value, Value> {
    match caller.request(&request(method, params)).await {
        Ok(reply) => Ok(serde_json::to_value(reply).unwrap()),
        Err(error) => Err(serde_json::to_value(error).unwrap()),
    }
}

/// What the terminal `at` names printed from `from` until it printed `marker`, and where it
/// got to.
async fn until_printed(caller: &Remote, at: &Value, from: u64, marker: &str) -> (String, u64) {
    let (mut seen, mut from) = (Vec::new(), from);
    for _ in 0..200 {
        let params = json!({"thread": at["thread"], "terminal": at["terminal"], "from": from,
            "wait_ms": 500});
        let read = asked(caller, "terminal/read", params).await.unwrap();
        let screen: Screenful = serde_json::from_value(read["screen"].clone()).unwrap();
        seen.extend_from_slice(screen.output.as_slice());
        from = screen.next;
        let text = String::from_utf8_lossy(&seen).into_owned();
        if text.contains(marker) {
            return (text, from);
        }
    }
    panic!(
        "never printed {marker:?}: {}",
        String::from_utf8_lossy(&seen)
    );
}

/// Types `line` and a return into the terminal `at` names.
async fn typed(caller: &Remote, at: &Value, line: &str) {
    let input = serde_json::to_value(Bytes::new(format!("{line}\r"))).unwrap();
    let params = json!({"thread": at["thread"], "terminal": at["terminal"], "input": input});
    asked(caller, "terminal/write", params).await.unwrap();
}

#[test]
fn the_person_s_server_opens_their_terminal_in_a_thread_s_folder_and_no_one_else_s() {
    let place = place("terminal-api");
    let model = FakeServer::start(Vec::new()).unwrap();
    choose(&place, "serve", "{ origin = \"http://localhost\" }");
    let home = place.home.to_string_lossy().into_owned();
    let env = [("SHELL", "/bin/bash"), ("HOME", home.as_str())];
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &env);
    let (_, port) = url.rsplit_once(':').unwrap();
    let origin = format!("http://localhost:{port}");
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
    block_on!(async {
        let owner = Remote::new(&url, token.trim().to_owned()).unwrap();
        let started = asked(&owner, "thread/start", json!({})).await.unwrap();
        assert_eq!(started["thread"]["terminals"], true, "{started}");
        let thread = started["thread"]["id"].clone();
        let size = json!({"cols": 80, "rows": 24});
        let params = json!({"thread": thread, "size": size});
        let opened = asked(&owner, "terminal/open", params).await.unwrap();
        let folder = opened["terminal"]["folder"].as_str().unwrap().to_owned();
        assert_eq!(started["thread"]["workspace"]["root"], folder.as_str());
        let at = json!({"thread": thread, "terminal": opened["terminal"]["id"]});

        // Typed, run in the thread's folder, and read back; resized as its shell sees.
        typed(&owner, &at, "printf 'hi-%s-%s\\n' \"${PWD##*/}\" $((40+2))").await;
        let name = folder.rsplit('/').next().unwrap();
        let (_, from) = until_printed(&owner, &at, 0, &format!("hi-{name}-42")).await;
        let size = json!({"cols": 100, "rows": 30});
        let resize = json!({"thread": thread, "terminal": at["terminal"], "size": size});
        asked(&owner, "terminal/resize", resize).await.unwrap();
        typed(&owner, &at, "stty size").await;
        until_printed(&owner, &at, from, "30 100").await;
        let listed = asked(&owner, "terminal/list", json!({"thread": thread})).await;
        assert_eq!(listed.unwrap()["terminals"][0]["id"], at["terminal"]);

        // Someone invited may open none: not on the server's computer, not in its owner's thread.
        let theirs = invited(&owner, &url, &origin).await;
        let own = asked(&theirs, "thread/start", json!({})).await.unwrap();
        assert!(own["thread"].get("terminals").is_none(), "{own}");
        let params = json!({"thread": own["thread"]["id"], "size": size});
        let refused = asked(&theirs, "terminal/open", params).await.unwrap_err();
        assert_eq!(refused["code"], -32012, "{refused}");
        let said = refused["message"].as_str().unwrap();
        assert!(said.contains("only on your own computer"), "{said}");
        let params = json!({"thread": thread, "terminal": at["terminal"], "from": 0});
        let refused = asked(&theirs, "terminal/read", params).await.unwrap_err();
        assert_eq!(refused["code"], -32001, "{refused}");

        let close = json!({"thread": thread, "terminal": at["terminal"]});
        asked(&owner, "terminal/close", close.clone())
            .await
            .unwrap();
        let gone = asked(&owner, "terminal/close", close).await.unwrap_err();
        assert_eq!(gone["code"], -32038, "{gone}");
        let listed = asked(&owner, "terminal/list", json!({"thread": thread})).await;
        assert_eq!(listed.unwrap()["terminals"], json!([]));
    });
}
