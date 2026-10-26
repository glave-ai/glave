use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, BufReader};

use super::*;

#[test]
fn only_https_or_this_computer_gets_a_sign_in() {
    assert!(Remote::new("http://glave.example.com", "t".into()).is_err());
    assert!(Remote::new("http://127.0.0.1:8080", "t".into()).is_ok());
}

#[test]
fn events_are_notifications_then_the_ending() {
    let thread = glave_event::ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
    let sent = Arc::new(Notification::Live {
        thread,
        update: crate::protocol::Update::Text { text: "hi".into() },
    });
    let Some(TurnItem::Note(note)) = item(&crate::rpc::notify(&sent)) else {
        panic!("a notification")
    };
    assert!(matches!(
        note.as_ref(),
        Notification::Live { update: crate::protocol::Update::Text { text }, .. } if text == "hi"
    ));
    assert!(item("not json").is_none());
    let failed = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"no thread"}}"#;
    let Some(TurnItem::Ended(Err(error))) = item(failed) else {
        panic!("an ending")
    };
    assert_eq!(error.code, code::NO_THREAD);
}

#[test]
fn an_answer_from_what_stands_in_front_of_the_server_says_its_status_and_how_it_starts() {
    let proxy = glave_ports::testing::FakeHttp::start(vec![
        (
            502,
            "<html>\n  <body>Bad gateway: the origin is down</body>\n</html>".into(),
        ),
        // Read by its shape, as a reply.
        (500, r#"{"message":"Internal error"}"#.into()),
    ])
    .unwrap();
    let remote = Remote::new(&proxy.url, "t".into()).unwrap();
    let said = || {
        let answered = glave_ports::block_on!(remote.request(&Request::Discover {}));
        answered.err().map(|error| error.message.to_string())
    };
    let check = "instead of glave: check that glave is running there and can be reached";
    assert_eq!(
        said(),
        Some(format!(
            "{} answered with HTTP 502 Bad Gateway {check} (it said: <html> <body>Bad gateway: \
             the origin is down</body> </html>)",
            proxy.url
        ))
    );
    assert_eq!(
        said(),
        Some(format!(
            "{} answered with HTTP 500 Internal Server Error {check} (it said: \
             {{\"message\":\"Internal error\"}})",
            proxy.url
        ))
    );
    assert_eq!(proxy.received().len(), 2);
}

#[test]
fn a_server_that_never_answers_a_request_is_given_up_at_its_limit() {
    let (asked, slow) = glave_ports::block_on!(async {
        // The clock jumps to each limit as soon as nothing else can run.
        tokio::time::pause();
        let (ours, _theirs) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(ours);
        let remote = Remote::over_lines(BufReader::new(read), write, "the server".into());
        let asked = remote.request(&Request::Discover {}).await;
        let slow = remote.request(&Request::ProviderRefresh {}).await;
        (asked, slow)
    });
    let said = |answered: Result<Reply, ApiError>| answered.err().map(|error| error.message);
    assert_eq!(
        said(asked).as_deref(),
        Some("the server did not answer server/discover within 2 minutes")
    );
    assert_eq!(
        said(slow).as_deref(),
        Some("the server did not answer provider/refresh within 15 minutes")
    );
}

#[test]
fn a_turn_whose_stream_goes_quiet_is_no_longer_followed_saying_so() {
    let ended = glave_ports::block_on!(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        // It starts a stream, then says nothing, not even the comment a live one sends.
        let quiet = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut asked = [0; 4096];
            let _ = stream.read(&mut asked).await;
            let head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
            stream.write_all(head).await.unwrap();
            std::future::pending::<()>().await;
        });
        let remote = Remote::new(&url, "t".into()).unwrap();
        let Ok(Sent::Started(mut turn)) = remote.start(&Request::Discover {}).await else {
            panic!("a turn followed");
        };
        tokio::time::pause();
        let ended = turn.next().await;
        quiet.abort();
        (ended, url)
    });
    let (Some(TurnItem::Ended(Err(error))), url) = ended else {
        panic!("the turn was still followed");
    };
    assert_eq!(
        error.message.to_string(),
        format!(
            "{url} sent nothing for 1 minute, so glave stopped following the turn; it goes on \
             there"
        )
    );
}
