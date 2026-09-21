//! The streaming acceptance test: an answer that arrives **while the engine is still running**.
//!
//! The report this file exists for is one sentence - *"claude code/cline e jemon thinking ki korse sob
//! kisu live dakha jai, aitare tamon kisu hosse nah, akbare answare disse"* - and the diagnosis was
//! that the daemon held a closed hand: `Engine::start` answered with a `Vec`, so a whole turn reached
//! the window when it was over. `src/sdcp/methods.rs` has the unit test for the turn loop; this is the
//! same property measured **over a real socket**, through `native_api`'s loopback path, which is the
//! transport a test can exercise without an API key or the public internet.
//!
//! The rule is about *timing*, not about text: the server writes one token, waits until the test has
//! seen it arrive in the sink, and only then writes the rest. A collector cannot pass that - it would
//! wait for the end of the body, the server would wait for the collector, and the test would fail on
//! its deadline rather than hang for ever.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use sdcd::engines::native_api::post_stream;
use sdcd::engines::{EngineEvent, EventSink};

/// A sink that reports every event on a channel, so the test can tell *when* it arrived.
fn probing_sink() -> (EventSink, Receiver<EngineEvent>) {
    let (sender, receiver) = channel::<EngineEvent>();

    let sink = EventSink::new(move |event: EngineEvent| {
        /* The receiver only goes away when the test is over, and a send that fails is not a turn's
        problem. */
        let _ = sender.send(event);
    });

    (sink, receiver)
}

/// Writes one **chunked** SSE frame and flushes it.
///
/// Chunked on purpose: `Transfer-Encoding: chunked` is what a Go or Node server sends for a stream of
/// unknown length, and reading such a body without decoding the framing is a turn that stops
/// mid-answer. This is the shape the daemon meets in the wild.
fn frame(stream: &mut TcpStream, payload: &str) -> std::io::Result<()> {
    let chunk = format!("{payload}\n\n");

    write!(stream, "{:x}\r\n{chunk}\r\n", chunk.len())?;

    stream.flush()
}

/// The one test this file is for: a token is in the sink while the body is still open.
#[test]
fn a_provider_that_streams_reaches_the_sink_before_the_body_ends() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the port");
    let (may_continue, wait_for_it) = channel::<()>();

    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("a client");
        let mut reader = BufReader::new(socket.try_clone().expect("a copy for the request"));
        let mut request = String::new();

        /* The request is drained before the answer is written, so the client's write cannot block on a
        full buffer. */
        while reader
            .read_line(&mut request)
            .map(|read| read > 0)
            .unwrap_or(false)
        {
            if request.ends_with("\r\n\r\n") {
                break;
            }
        }

        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            )
            .expect("the head");
        socket.flush().expect("the head");

        /* The first token, and then a wait. A collector is still sitting inside `post_stream` with an
        empty sink, so this wait is where the test's five seconds run out - which is the failure this
        file is written to produce. */
        frame(
            &mut socket,
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}",
        )
        .expect("the first frame");

        let _ = wait_for_it.recv_timeout(Duration::from_secs(5));

        frame(
            &mut socket,
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}",
        )
        .expect("the second frame");
        frame(&mut socket, "data: [DONE]").expect("the sentinel");

        /* The terminating chunk, then the close. */
        let _ = socket.write_all(b"0\r\n\r\n");
        let _ = socket.flush();
    });

    let (sink, seen) = probing_sink();
    let url = format!("http://{address}/v1/chat/completions");

    /* The request blocks for as long as the body is open, so it runs on a thread of its own and the
    assertions read what the sink was given. */
    let client = std::thread::spawn(move || post_stream(&url, &[], "{}", &sink));

    let first = seen
        .recv_timeout(Duration::from_secs(5))
        .expect("the first delta never reached the sink while the body was still open");

    assert!(
        matches!(first, EngineEvent::Delta(ref text) if text == "Hel"),
        "{first:?}"
    );

    /* The daemon has the token, so the server may finish - which is what makes the rest of the
    stream, and this test, possible at all. */
    may_continue.send(()).expect("the server is gone");

    let mut rest = Vec::new();

    while let Ok(event) = seen.recv_timeout(Duration::from_secs(5)) {
        let terminal = event.is_terminal();

        rest.push(event);

        if terminal {
            break;
        }
    }

    assert!(
        matches!(
            rest.as_slice(),
            [EngineEvent::Delta(second), EngineEvent::Done { .. }] if second == "lo"
        ),
        "{rest:?}"
    );

    client
        .join()
        .expect("the client thread")
        .expect("the request succeeded");
    server.join().expect("the server thread");
}

/// And the same call against a server that refuses: a rejection is not a stream, so it comes back as
/// the error of the call - in the provider's own words.
#[test]
fn a_rejected_request_is_an_error_with_the_providers_sentence() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the port");

    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("a client");
        let mut reader = BufReader::new(socket.try_clone().expect("a copy for the request"));
        let mut request = String::new();

        /* The request is drained first. A socket closed with bytes still unread resets the connection on
        Windows, which would hide the sentence this test is about behind an OS error. */
        while reader
            .read_line(&mut request)
            .map(|read| read > 0)
            .unwrap_or(false)
        {
            if request.ends_with("\r\n\r\n") {
                break;
            }
        }

        let body = r#"{"error":{"message":"invalid x-api-key"}}"#;

        let _ = write!(
            socket,
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.flush();
    });

    let reason = post_stream(
        &format!("http://{address}/v1/messages"),
        &[],
        /* An empty body on purpose: the server below answers and closes without reading a request
        body, and on Windows a socket closed with unread bytes in its buffer resets the connection
        instead of ending it cleanly - which would hide the sentence this test is about. */
        "",
        &EventSink::discarding(),
    )
    .expect_err("a 401 is not a stream");

    assert_eq!(reason, "invalid x-api-key (401)");

    server.join().expect("the server thread");
}
