//! The `/api/stream` subscriber against FakeWorkman: frames, a dropped connection, the reconnect with
//! `Last-Event-ID`, a stop while the server is silent, and the backoff while unreachable.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use async_channel::Receiver;
use supraman_workman::fake::FakeWorkman;
use supraman_workman::{Client, Stream, StreamConfig, StreamEvent, StreamSignal};

const TOKEN: &str = "fake-reader-token";

fn config() -> StreamConfig {
    StreamConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        connect_timeout: Duration::from_secs(2),
        poll: Duration::from_millis(50),
    }
}

#[track_caller]
fn next(rx: &Receiver<StreamSignal>) -> StreamSignal {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match rx.try_recv() {
            Ok(signal) => return signal,
            Err(e) if e.is_closed() || Instant::now() > until => panic!("no stream signal: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// Pushes `frame` once the stream has registered with the fake.
fn push_when_open(fake: &FakeWorkman, frame: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.push_frame(frame) == 0 {
        assert!(Instant::now() < deadline, "the stream never registered");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn frames_reach_a_subscriber_and_a_dropped_stream_resumes_from_the_last_id() {
    let fake = FakeWorkman::start(Vec::new());
    let client = Client::new(fake.base(), TOKEN);
    let (stream, rx) = Stream::subscribe_with(&client, config());
    assert_eq!(next(&rx), StreamSignal::Connected);
    push_when_open(&fake, "id: 41\ndata: {\"seq\":7,\"runId\":\"r1\",\"itemId\":null,\"kind\":\"transition\"}\n\n");
    assert_eq!(next(&rx), StreamSignal::Event(StreamEvent { seq: 7, run_id: Some("r1".into()), item_id: None, kind: "transition".into() }));

    fake.drop_streams();
    assert!(matches!(next(&rx), StreamSignal::Lost(_)));
    assert_eq!(next(&rx), StreamSignal::Connected);
    let streams: Vec<_> = fake.seen().into_iter().filter(|s| s.target == "/api/stream").collect();
    assert_eq!(streams.len(), 2, "{streams:?}");
    assert!(streams.iter().all(|s| s.method == "GET" && s.authorization.as_deref() == Some("Bearer fake-reader-token") && s.header("accept") == Some("text/event-stream")));
    assert_eq!((streams[0].header("last-event-id"), streams[1].header("last-event-id")), (None, Some("41")), "the reconnect resumes from the last id");

    // the fake is silent now: stop must not wait for it
    let t = Instant::now();
    stream.stop();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn stream_backs_off_while_unreachable_and_never_says_the_token() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let client = Client::new(format!("http://{free}"), TOKEN);
    // Windows takes ~2 s to refuse a connect to a closed port.
    let config = StreamConfig { connect_timeout: Duration::from_millis(300), ..config() };
    let (stream, rx) = Stream::subscribe_with(&client, config);
    let mut at = Vec::new();
    for _ in 0..4 {
        match next(&rx) {
            StreamSignal::Lost(r) => {
                assert!(!r.contains(TOKEN), "{r}");
                at.push(Instant::now());
            }
            other => panic!("{other:?}"),
        }
    }
    // 50, 100, 200 ms between attempts (capped at 200): the last gap is the widest.
    let gaps: Vec<Duration> = at.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps[2] >= Duration::from_millis(180), "{gaps:?}");
    drop(stream);
}
