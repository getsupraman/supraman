use std::time::{Duration, Instant};

use async_channel::Receiver;
use supraman_workman::fake::{Answer, FakeWorkman};
use supraman_workman::{Client, Stream, StreamConfig, StreamEvent, StreamSignal};

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

#[test]
fn a_changed_route_answers_the_next_request() {
    let fake = FakeWorkman::start(vec![("/api/v1/resolve?ref=a".into(), Answer::json(r#"{"seq":1}"#))]);
    let client = Client::new(fake.base(), "fake-token");
    assert_eq!(client.resolve("a").unwrap().seq, 1);
    fake.set("/api/v1/resolve?ref=a", Answer::json(r#"{"seq":2}"#));
    fake.set("/api/v1/resolve?ref=b", Answer::json(r#"{"seq":3}"#));
    assert_eq!((client.resolve("a").unwrap().seq, client.resolve("b").unwrap().seq), (2, 3));
}

#[test]
fn a_stream_answer_without_the_v1_header_is_refused() {
    let fake = FakeWorkman::start(Vec::new());
    let client = Client::new(fake.base(), "fake-token");
    let config = StreamConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        connect_timeout: Duration::from_secs(2),
        poll: Duration::from_millis(50),
    };
    fake.set("/api/stream", Answer::text(": connected\n\n").api(None));
    let (stream, rx) = Stream::subscribe_with(&client, config);
    assert_eq!(
        next(&rx),
        StreamSignal::Lost("the stream answered without X-Workman-Api: not a Workman read API v1".into())
    );
    stream.stop();

    fake.set("/api/stream", Answer::text(": connected\n\n").api(Some("2")));
    let (stream, rx) = Stream::subscribe_with(&client, config);
    assert_eq!(next(&rx), StreamSignal::Lost("the stream speaks read API 2, Supraman reads v1".into()));
    stream.stop();
}

#[test]
fn pushed_frames_reach_a_subscriber_and_a_dropped_stream_reconnects() {
    let fake = FakeWorkman::start(Vec::new());
    let client = Client::new(fake.base(), "fake-token");
    let config = StreamConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        connect_timeout: Duration::from_secs(2),
        poll: Duration::from_millis(50),
    };
    let (stream, rx) = Stream::subscribe_with(&client, config);
    assert_eq!(next(&rx), StreamSignal::Connected);
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.push(r#"{"seq":7,"runId":"r1","itemId":null,"kind":"transition"}"#) == 0 {
        assert!(Instant::now() < deadline, "the stream never registered");
        std::thread::sleep(Duration::from_millis(20));
    }
    let event = StreamEvent { seq: 7, run_id: Some("r1".into()), item_id: None, kind: "transition".into() };
    assert_eq!(next(&rx), StreamSignal::Event(event));

    fake.drop_streams();
    assert!(matches!(next(&rx), StreamSignal::Lost(_)));
    assert_eq!(next(&rx), StreamSignal::Connected);
    stream.stop();
    let seen = fake.seen();
    assert_eq!(seen.iter().filter(|s| s.target == "/api/stream").count(), 2, "{seen:?}");
    assert!(seen.iter().all(|s| s.method == "GET" && s.authorization.as_deref() == Some("Bearer fake-token")));
}
