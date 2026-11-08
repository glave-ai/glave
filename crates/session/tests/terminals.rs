//! A person's terminals through the session on their own computer, as a server holds it
//! (`Waking`): the thread says one opens there; listing them never wakes the computer, opening
//! one does; a computer that opens none says so for good, never "try again"; a terminal it
//! doesn't have is not found; and once the thread is archived none is listed, read, typed into
//! or resized, nor one opened. A computer of this machine stands in for the cluster's, with no
//! glave of its own to start a shell, as the cli's tests have.

use glave_ports::computer::{Bytes, TerminalId, TerminalSize};
use glave_session::protocol::{Reply, Request};

use crate::homes::{served_with_homes, started};
use crate::support::*;

const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };

#[test]
fn a_thread_on_its_person_s_computer_reaches_its_terminals_there_until_it_is_archived() {
    let dir = folder("session-terminals-home");
    block_on!(async {
        let (homes, server, _) = served_with_homes(&dir, &Shared::default()).await;
        let summary = started(&server, None).await;
        assert!(
            summary.terminals,
            "a terminal opens on its person's computer"
        );
        let thread = summary.id;
        let listed = server.request(Request::TerminalList { thread }).await;
        assert!(
            matches!(&listed, Ok(Reply::Terminals { terminals }) if terminals.is_empty()),
            "{listed:?}"
        );
        assert_eq!(homes.woken(), 0, "listing never wakes the computer");

        let open = Request::TerminalOpen { thread, size: SIZE };
        let refused = server.request(open).await.unwrap_err();
        assert_eq!(homes.woken(), 1, "opening one wakes it");
        assert_eq!(refused.code, -32022, "{refused:?}");
        assert!(!refused.message.contains("Try again"), "{refused:?}");
        let listed = server.request(Request::TerminalList { thread }).await;
        assert!(matches!(listed, Ok(Reply::Terminals { .. })), "{listed:?}");
        let terminal = TerminalId::from_uuid(uuid::Uuid::nil());
        let read = Request::TerminalRead {
            thread,
            terminal,
            from: 0,
            wait_ms: 0,
        };
        let missing = server.request(read.clone()).await.unwrap_err();
        assert_eq!(missing.code, -32038, "{missing:?}");

        server
            .request(Request::ThreadArchive { thread })
            .await
            .unwrap();
        let input = Bytes::new(b"ls\r".to_vec());
        for request in [
            Request::TerminalList { thread },
            read,
            Request::TerminalWrite {
                thread,
                terminal,
                input,
            },
            Request::TerminalResize {
                thread,
                terminal,
                size: SIZE,
            },
            Request::TerminalOpen { thread, size: SIZE },
        ] {
            let refused = server.request(request).await.unwrap_err();
            assert_eq!(refused.code, -32014, "{refused:?}");
        }
        let close = Request::TerminalClose { thread, terminal };
        let gone = server.request(close).await.unwrap_err();
        assert_eq!(gone.code, -32038, "closing is still asked of its computer");
        assert_eq!(homes.woken(), 1);
    });
}
