//! glave first on the `PATH` of a command the sandbox confines, beside the person's own
//! server: the command reaches this glave, and can no more act as the person than any other
//! program in the sandbox, over HTTP or through the glave command line.

use std::ffi::OsString;
use std::path::PathBuf;

use glave_computer::{CancelSwitch, LocalComputers};
use glave_ports::block_on;
use glave_ports::computer::{
    Command, Computers, Ending, Environment, Sandbox, SandboxLevel, Sources,
};
use glave_ports::testing::Captured;
use glave_provider_openai::fake::FakeServer;

use crate::http::serve_http;
use crate::support::place;

/// The person's server runs; a sandboxed command with the network on and this glave on its
/// `PATH` tries to approve over HTTP, with no token and with whatever it can read of the
/// server's, and asks the glave command line. Each is refused before any approval is looked
/// up, as glave's folder, where the token and the server's socket are, stays hidden from it.
#[test]
fn a_sandboxed_command_with_glave_on_its_path_cannot_act_as_the_person() {
    if !glave_computer::sandbox_available() {
        assert!(
            std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
            "GLAVE_REQUIRE_SANDBOX is set but the sandbox cannot run here"
        );
        return;
    }
    let place = place("onpath");
    let model = FakeServer::start(Vec::new()).unwrap();
    let (_served, url) = serve_http(&place, &model, &[]);
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
    let dir = place.home.parent().unwrap().to_path_buf();
    let on_path = place.home.join("bin");
    let glave = PathBuf::from(env!("CARGO_BIN_EXE_glave"));
    // glave's folder named, as the server's commands have it: glave then keeps its key in a
    // file there, never in this machine's keychain.
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([
            ("HOME".into(), dir.clone().into_os_string()),
            ("GLAVE_HOME".into(), place.home.clone().into_os_string()),
        ])
        .collect();
    let script = tries(&url);
    block_on!(async {
        glave_computer::glave_on_path(&on_path, &glave)
            .await
            .unwrap();
        let computers =
            LocalComputers::new(dir.join("copies"), environment).with_glave_on_path(&on_path);
        let computer = computers.create(&Environment::host()).await.unwrap();
        let project = Sources::Folder {
            path: place.project.to_string_lossy().into_owned(),
        };
        let here = computers.open(computer.id, &project).await.unwrap().id;
        let mut command = Command::new(script);
        command.sandbox = Some(Sandbox {
            level: SandboxLevel::Project,
            network: true,
            home: Some(dir.clone()),
            glave_home: Some(place.home.clone()),
        });
        command.timeout_secs = Some(60);
        let output = Captured::default();
        let finished = computers
            .run(here, &command, &output, &CancelSwitch::default())
            .await
            .unwrap();
        let said = output.text();
        assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{said}");
        let reached = on_path.join("glave");
        assert!(said.contains(&reached.display().to_string()), "{said}");
        assert!(said.contains("version-ran"), "{said}");
        assert!(said.contains("approve=401"), "{said}");
        assert!(said.contains("with=401"), "{said}");
        assert!(said.contains("read=[]"), "{said}");
        assert!(!said.contains(token.trim()), "the token was read");
        assert!(!said.contains("cli=0"), "{said}");
        // The person, with the token, still reaches the server.
        let answered = reqwest::Client::new()
            .post(format!("{url}/"))
            .bearer_auth(token.trim())
            .header("content-type", "application/json")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"thread/list","params":{}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(answered.status(), 200);
    });
}

/// What the sandboxed command tries: finds and runs `glave`, then asks to approve over HTTP,
/// with no token and with what it can read of the server's, and runs the glave command line.
fn tries(url: &str) -> String {
    let decide = r#"{"jsonrpc":"2.0","id":1,"method":"approval/decide","params":{"thread":"t","approval":"a","decision":"allow_once"}}"#;
    let post = |auth: &str| {
        format!(
            "curl -s -o /dev/null -w '%{{http_code}}' -X POST '{url}/' \
             -H 'content-type: application/json' {auth} --data '{decide}'"
        )
    };
    format!(
        "command -v glave; glave --version >/dev/null && echo version-ran; \
         echo \"approve=$({bare})\"; \
         read=$(cat \"$GLAVE_HOME/serve.token\" 2>/dev/null); echo \"read=[$read]\"; \
         echo \"with=$({with})\"; \
         glave tasks >/dev/null 2>&1; echo \"cli=$?\"",
        bare = post(""),
        with = post("-H \"authorization: Bearer $read\""),
    )
}
