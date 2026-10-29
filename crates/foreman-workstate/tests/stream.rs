//! The `/api/stream` subscriber against a fake SSE server: frames, a dropped connection, a refused
//! one, reconnects with `Last-Event-ID`, and a stop while the server is silent.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use async_channel::Receiver;
use foreman_workstate::{Client, Stream, StreamConfig, StreamEvent, StreamSignal};

const TOKEN: &str = "fake-reader-token";

/// The request head of one connection: its request line and headers, lower-cased names.
fn read_head(stream: &TcpStream) -> (String, Vec<(String, String)>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let Some((k, v)) = h.trim_end().split_once(':') else { break };
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    (line.trim_end().to_string(), headers)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn chunk(s: &str) -> String {
    format!("{:x}\r\n{s}\r\n", s.len())
}

fn config() -> StreamConfig {
    StreamConfig {
        initial_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
        connect_timeout: Duration::from_secs(2),
        poll: Duration::from_millis(50),
    }
}

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

fn event(seq: i64, kind: &str) -> StreamSignal {
    StreamSignal::Event(StreamEvent {
        seq,
        run_id: Some("r1".into()),
        item_id: None,
        kind: kind.into(),
    })
}

#[test]
fn stream_delivers_frames_and_reconnects_after_a_drop_and_a_refusal() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (heads_tx, heads) = mpsc::channel();
    let (release_tx, release) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        // 1: chunked, frames split across chunks, a comment, an id, a multi-line data, then dropped.
        let (mut s, _) = listener.accept().unwrap();
        heads_tx.send(read_head(&s)).unwrap();
        let frames = [
            ": connected\n\n",
            "id: 41\ndata: {\"seq\":1235,\"runId\":\"r1\",",
            "\"itemId\":null,\"kind\":\"transition\",\"addedLater\":1}\n\n",
            "event: change\ndata: {\"seq\":1236,\"runId\":\"r1\",\ndata: \"kind\":\"note\"}\n\ndata: not an event\n\n",
        ];
        write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        for f in frames {
            s.write_all(chunk(f).as_bytes()).unwrap();
            s.flush().unwrap();
        }
        drop(s);
        // 2: refused.
        let (mut s, _) = listener.accept().unwrap();
        heads_tx.send(read_head(&s)).unwrap();
        write!(s, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        drop(s);
        // 3: close-delimited, one frame, then silent until the test is done.
        let (mut s, _) = listener.accept().unwrap();
        heads_tx.send(read_head(&s)).unwrap();
        write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nid: 42\r\ndata: {{\"seq\":1240,\"runId\":\"r1\",\"kind\":\"claim\"}}\r\n\r\n").unwrap();
        s.flush().unwrap();
        let _ = release.recv();
    });

    let client = Client::new(&base, TOKEN);
    let (stream, rx) = Stream::subscribe_with(&client, config());
    assert_eq!(next(&rx), StreamSignal::Connected);
    assert_eq!(next(&rx), event(1235, "transition"));
    assert_eq!(next(&rx), event(1236, "note"), "data lines join with \\n");
    assert!(matches!(next(&rx), StreamSignal::Lost(r) if r == "the stream closed"), "dropped mid-stream");
    assert!(matches!(next(&rx), StreamSignal::Lost(r) if r == "WorkState answered HTTP 401"));
    assert_eq!(next(&rx), StreamSignal::Connected);
    assert_eq!(next(&rx), event(1240, "claim"));

    let (line, h) = heads.recv().unwrap();
    assert_eq!(line, "GET /api/stream HTTP/1.1");
    assert_eq!(header(&h, "authorization"), Some("Bearer fake-reader-token"));
    assert_eq!(header(&h, "accept"), Some("text/event-stream"));
    assert_eq!(header(&h, "last-event-id"), None);
    let (_, h) = heads.recv().unwrap();
    assert_eq!(header(&h, "last-event-id"), Some("41"), "the reconnect resumes from the last id");
    let (_, h) = heads.recv().unwrap();
    assert_eq!(header(&h, "last-event-id"), Some("41"));

    // The server is silent now: stop must not wait for it.
    let t = Instant::now();
    stream.stop();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    release_tx.send(()).unwrap();
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
