//! A fake WorkState read API on 127.0.0.1 for tests (feature `fake`): answers each request target
//! from a table the test can change, holds `/api/stream` open as SSE so a test can push frames, and
//! records what it was asked. Never the real server (C21).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const STREAM_TARGET: &str = "/api/stream";

/// One request as the fake read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    pub authorization: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub content_type: &'static str,
    /// The `X-WorkState-Api` value; `None` sends no such header.
    pub api: Option<&'static str>,
    pub body: String,
}

impl Answer {
    pub fn json(body: impl Into<String>) -> Self {
        Self { status: 200, content_type: "application/json", api: Some("1"), body: body.into() }
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self { content_type: "text/plain; charset=utf-8", ..Self::json(body) }
    }

    pub fn status(status: u16, body: impl Into<String>) -> Self {
        Self { status, ..Self::json(body) }
    }

    pub fn api(self, api: Option<&'static str>) -> Self {
        Self { api, ..self }
    }
}

type Routes = Arc<Mutex<Vec<(String, Answer)>>>;

pub struct FakeWorkState {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    routes: Routes,
    /// Open `/api/stream` connections, past their response head.
    streams: Arc<Mutex<Vec<TcpStream>>>,
}

impl FakeWorkState {
    /// `routes`: exact request target (path and query) → answer. Anything else is a v1 404, except
    /// `/api/stream`, which stays open until [`Self::push`] fails on it or [`Self::drop_streams`].
    pub fn start(routes: Vec<(String, Answer)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a fake WorkState");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let routes: Routes = Arc::new(Mutex::new(routes));
        let streams: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let (table, log, open) = (routes.clone(), seen.clone(), streams.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (routes, log, open) = (table.clone(), log.clone(), open.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let mut authorization = None;
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).is_err() {
                            return;
                        }
                        let Some((k, v)) = h.trim_end().split_once(':') else { break };
                        if k.eq_ignore_ascii_case("authorization") {
                            authorization = Some(v.trim().to_string());
                        }
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let target = parts.next().unwrap_or("").to_string();
                    let answer = routes
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(t, _)| *t == target)
                        .map(|(_, a)| a.clone());
                    let stream_open = answer.is_none() && target == STREAM_TARGET;
                    let answer = answer.unwrap_or_else(|| Answer::status(404, format!(r#"{{"error":"no route {target}"}}"#)));
                    log.lock().unwrap().push(Seen { method, target, authorization });
                    if stream_open {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nX-WorkState-Api: 1\r\n\r\n: connected\n\n";
                        if stream.write_all(head.as_bytes()).is_ok() {
                            open.lock().unwrap().push(stream);
                        }
                        return;
                    }
                    let api = answer.api.map(|v| format!("X-WorkState-Api: {v}\r\n")).unwrap_or_default();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {} X\r\nContent-Type: {}\r\n{api}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        answer.status,
                        answer.content_type,
                        answer.body.len(),
                        answer.body
                    );
                });
            }
        });
        Self { base, seen, routes, streams }
    }

    /// Answers `target` with `answer` from now on.
    pub fn set(&self, target: impl Into<String>, answer: Answer) {
        let target = target.into();
        let mut routes = self.routes.lock().unwrap();
        match routes.iter_mut().find(|(t, _)| *t == target) {
            Some(route) => route.1 = answer,
            None => routes.push((target, answer)),
        }
    }

    /// One SSE frame `data: <data>` to every open stream; how many took it.
    pub fn push(&self, data: &str) -> usize {
        let frame = format!("data: {data}\n\n");
        let mut streams = self.streams.lock().unwrap();
        streams.retain_mut(|s| s.write_all(frame.as_bytes()).and_then(|()| s.flush()).is_ok());
        streams.len()
    }

    /// Open streams, as of the last [`Self::push`].
    pub fn streams(&self) -> usize {
        self.streams.lock().unwrap().len()
    }

    /// Closes every open stream: the subscriber sees the connection end.
    pub fn drop_streams(&self) {
        for s in self.streams.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The URL a `settings.toml` would name: `<base>/mcp`.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
