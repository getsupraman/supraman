//! `GET /api/stream`. An event means "re-read"; `Connected` after a `Lost` means "re-read everything", since
//! frames sent in between are gone. SSE as the WHATWG spec reads it, except `retry:`: the backoff is ours.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};

use crate::client::{API_HEADER, API_VERSION, Client};
use crate::model::StreamEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamSignal {
    /// The server answered 200; anything before may have been missed.
    Connected,
    Event(StreamEvent),
    /// The connection failed or ended; a reconnect follows after the backoff. Never the token.
    Lost(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    /// The first wait after a loss; doubled per failed attempt, reset by a 200.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub connect_timeout: Duration,
    /// How often a blocked read wakes to see whether it was stopped.
    pub poll: Duration,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(5),
            poll: Duration::from_millis(200),
        }
    }
}

/// Dropping it stops the thread, as does dropping the receiver (at the next signal).
pub struct Stream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Stream {
    pub fn subscribe(client: &Client) -> (Self, Receiver<StreamSignal>) {
        Self::subscribe_with(client, StreamConfig::default())
    }

    pub fn subscribe_with(client: &Client, config: StreamConfig) -> (Self, Receiver<StreamSignal>) {
        let (tx, rx) = async_channel::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            client: client.clone(),
            config,
            stop: stop.clone(),
            tx,
            last_id: None,
        };
        let thread = std::thread::Builder::new()
            .name("workstate-stream".into())
            .spawn(move || worker.run())
            .expect("spawn the WorkState stream thread");
        (Self { stop, thread: Some(thread) }, rx)
    }

    /// Stops the thread and waits for it (at most one poll, or one connect timeout).
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.halt();
    }
}

struct Worker {
    /// Asked for a bearer at each connect: an access token refreshed meanwhile is used.
    client: Client,
    config: StreamConfig,
    stop: Arc<AtomicBool>,
    tx: Sender<StreamSignal>,
    last_id: Option<String>,
}

/// Why a connection ended: the receiver is gone or we were stopped (`Quit`), or anything else.
enum End {
    Quit,
    Lost(String),
}

impl Worker {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn run(mut self) {
        let mut backoff = self.config.initial_backoff;
        loop {
            let reason = match self.connect(&mut backoff) {
                End::Quit => return,
                End::Lost(reason) => reason,
            };
            if self.stopped() || self.tx.send_blocking(StreamSignal::Lost(reason)).is_err() {
                return;
            }
            let until = Instant::now() + backoff;
            while Instant::now() < until {
                if self.stopped() {
                    return;
                }
                std::thread::sleep(self.config.poll.min(until.saturating_duration_since(Instant::now())));
            }
            backoff = (backoff * 2).min(self.config.max_backoff);
        }
    }

    fn connect(&mut self, backoff: &mut Duration) -> End {
        let base = self.client.base().to_string();
        let (tls, rest) = match (base.strip_prefix("http://"), base.strip_prefix("https://")) {
            (Some(rest), _) => (false, rest),
            (_, Some(rest)) => (true, rest),
            _ => return End::Lost(format!("{base} is not http or https")),
        };
        let token = match self.client.bearer() {
            Ok(token) => token,
            Err(e) => return End::Lost(e.to_string()),
        };
        let (authority, prefix) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
        let prefix = if prefix.is_empty() { String::new() } else { format!("/{}", prefix.trim_end_matches('/')) };
        let host = authority.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).map_or(authority, |(h, _)| h);
        let dial = if authority.contains(':') { authority.to_string() } else { format!("{authority}:{}", if tls { 443 } else { 80 }) };
        let addr = match dial.to_socket_addrs().map(|mut a| a.next()) {
            Ok(Some(addr)) => addr,
            Ok(None) => return End::Lost(format!("{authority} has no address")),
            Err(e) => return End::Lost(format!("{authority}: {e}")),
        };
        let tcp = match TcpStream::connect_timeout(&addr, self.config.connect_timeout) {
            Ok(s) => s,
            Err(e) => return End::Lost(format!("{base} is unreachable ({e})")),
        };
        let _ = tcp.set_read_timeout(Some(self.config.connect_timeout));
        let mut socket = if tls {
            let connector = match native_tls::TlsConnector::new() {
                Ok(c) => c,
                Err(e) => return End::Lost(format!("no TLS ({e})")),
            };
            match connector.connect(host, tcp) {
                Ok(s) => Conn::Tls(Box::new(s)),
                Err(e) => return End::Lost(format!("{base}: TLS failed ({e})")),
            }
        } else {
            Conn::Plain(tcp)
        };
        socket.set_read_timeout(self.config.poll);
        let mut request = format!(
            "GET {prefix}/api/stream HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\n"
        );
        if let Some(id) = &self.last_id {
            request.push_str(&format!("Last-Event-ID: {id}\r\n"));
        }
        request.push_str("\r\n");
        if let Err(e) = socket.write_all(request.as_bytes()) {
            return End::Lost(format!("cannot send the request ({e})"));
        }

        let mut http = HttpReader::default();
        let mut sse = SseParser::default();
        let mut buf = [0u8; 8192];
        loop {
            if self.stopped() {
                return End::Quit;
            }
            let n = match socket.read(&mut buf) {
                Ok(0) => return End::Lost("the stream closed".into()),
                Ok(n) => n,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => continue,
                Err(e) => return End::Lost(format!("the stream broke ({e})")),
            };
            let (body, head) = match http.feed(&buf[..n]) {
                Ok(x) => x,
                Err(reason) => {
                    if http.status == Some(401) {
                        self.client.rejected(&token);
                    }
                    return End::Lost(crate::client::scrub(&token, &reason));
                }
            };
            if head {
                *backoff = self.config.initial_backoff;
                if self.tx.send_blocking(StreamSignal::Connected).is_err() {
                    return End::Quit;
                }
            }
            let frames = match sse.feed(&body) {
                Ok(frames) => frames,
                Err(reason) => return End::Lost(reason),
            };
            for data in frames {
                // A frame that is not an event (a hello, a future shape) is not ours to report.
                if let Ok(event) = serde_json::from_str::<StreamEvent>(&data)
                    && self.tx.send_blocking(StreamSignal::Event(event)).is_err()
                {
                    return End::Quit;
                }
            }
            if let Some(id) = sse.last_id.clone() {
                self.last_id = Some(id);
            }
            if http.done() {
                return End::Lost("the stream ended".into());
            }
        }
    }
}

enum Conn {
    Plain(TcpStream),
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Conn {
    fn set_read_timeout(&self, poll: Duration) {
        let tcp = match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => s.get_ref(),
        };
        let _ = tcp.set_read_timeout(Some(poll));
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

/// The response head, then the body: chunked, `Content-Length`, or until close.
#[derive(Default)]
struct HttpReader {
    head: Vec<u8>,
    body: Option<Body>,
    status: Option<u16>,
}

enum Body {
    Close,
    Length(usize),
    /// Bytes left of the current chunk, or reading a size line.
    Chunked(Chunk),
    Done,
}

enum Chunk {
    Size(Vec<u8>),
    Data(usize),
    /// The CRLF after a chunk's data; how many of its bytes are left.
    End(u8),
}

impl HttpReader {
    fn done(&self) -> bool {
        matches!(self.body, Some(Body::Done))
    }

    /// Body bytes decoded from `bytes`, and whether the head completed in this call.
    fn feed(&mut self, bytes: &[u8]) -> Result<(Vec<u8>, bool), String> {
        let mut out = Vec::new();
        if self.body.is_none() {
            self.head.extend_from_slice(bytes);
            let Some(end) = self.head.windows(4).position(|w| w == b"\r\n\r\n") else {
                if self.head.len() > MAX_LINE {
                    return Err("the response head is too long".into());
                }
                return Ok((out, false));
            };
            let head = String::from_utf8_lossy(&self.head[..end]).into_owned();
            let mut lines = head.split("\r\n");
            let status = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse::<u16>().ok());
            self.status = status;
            match status {
                Some(200) => {}
                Some(status) => return Err(format!("WorkState answered HTTP {status}")),
                None => return Err("WorkState's answer was not HTTP".into()),
            }
            let mut body = Body::Close;
            let mut api = None;
            for line in lines {
                let Some((k, v)) = line.split_once(':') else { continue };
                let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
                if k.eq_ignore_ascii_case(API_HEADER) {
                    api = Some(v.to_string());
                } else if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                    body = Body::Chunked(Chunk::Size(Vec::new()));
                } else if k == "content-length" && !matches!(body, Body::Chunked(_)) {
                    body = Body::Length(v.parse().map_err(|_| "a bad Content-Length".to_string())?);
                }
            }
            match api.as_deref() {
                Some(API_VERSION) => {}
                Some(v) => return Err(format!("the stream speaks read API {v}, Foreman reads v{API_VERSION}")),
                None => return Err(format!("the stream answered without {API_HEADER}: not a WorkState read API v{API_VERSION}")),
            }
            let tail = self.head.split_off(end + 4);
            self.head.clear();
            self.body = Some(body);
            out.extend(self.decode(&tail)?);
            return Ok((out, true));
        }
        out.extend(self.decode(bytes)?);
        Ok((out, false))
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let (decoded, used) = self.step(rest)?;
            out.extend_from_slice(decoded);
            rest = &rest[used..];
        }
        Ok(out)
    }

    /// One state's worth of `bytes`: the body bytes it yields and how many input bytes it used.
    fn step<'a>(&mut self, bytes: &'a [u8]) -> Result<(&'a [u8], usize), String> {
        let body = self.body.as_mut().expect("head read");
        match body {
            Body::Close => Ok((bytes, bytes.len())),
            Body::Done => Ok((&[], bytes.len())),
            Body::Length(left) => {
                let n = (*left).min(bytes.len());
                *left -= n;
                if *left == 0 {
                    *body = Body::Done;
                }
                Ok((&bytes[..n], n))
            }
            Body::Chunked(chunk) => match chunk {
                Chunk::Size(line) => {
                    let Some(nl) = bytes.iter().position(|&b| b == b'\n') else {
                        line.extend_from_slice(bytes);
                        return Ok((&[], bytes.len()));
                    };
                    line.extend_from_slice(&bytes[..nl]);
                    let text = String::from_utf8_lossy(line);
                    let hex = text.trim().split(';').next().unwrap_or("").trim();
                    let size = usize::from_str_radix(hex, 16).map_err(|_| "a bad chunk size".to_string())?;
                    *body = if size == 0 { Body::Done } else { Body::Chunked(Chunk::Data(size)) };
                    Ok((&[], nl + 1))
                }
                Chunk::Data(left) => {
                    let n = (*left).min(bytes.len());
                    *left -= n;
                    if *left == 0 {
                        *chunk = Chunk::End(2);
                    }
                    Ok((&bytes[..n], n))
                }
                Chunk::End(left) => {
                    let n = (*left as usize).min(bytes.len());
                    *left -= n as u8;
                    if *left == 0 {
                        *chunk = Chunk::Size(Vec::new());
                    }
                    Ok((&[], n))
                }
            },
        }
    }
}

/// The longest response head, and the longest event a stream may send.
const MAX_LINE: usize = 64 * 1024;

/// Lines in, the data of each dispatched event out.
#[derive(Default)]
struct SseParser {
    line: Vec<u8>,
    /// The previous byte was a `\r`: a `\n` right after it ends nothing.
    after_cr: bool,
    data: Option<String>,
    last_id: Option<String>,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' && self.after_cr {
                self.after_cr = false;
                continue;
            }
            self.after_cr = b == b'\r';
            if b == b'\r' || b == b'\n' {
                let line = String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned();
                if let Some(data) = self.line_done(&line) {
                    out.push(data);
                }
            } else {
                self.line.push(b);
            }
            if self.line.len() + self.data.as_ref().map_or(0, String::len) > MAX_LINE {
                return Err(format!("an event over {} KiB", MAX_LINE / 1024));
            }
        }
        Ok(out)
    }

    fn line_done(&mut self, line: &str) -> Option<String> {
        if line.is_empty() {
            return self.data.take();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
        match field {
            "data" => match &mut self.data {
                Some(d) => {
                    d.push('\n');
                    d.push_str(value);
                }
                None => self.data = Some(value.to_string()),
            },
            "id" if !value.contains('\0') => self.last_id = Some(value.to_string()),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(chunks: &[&[u8]]) -> (Vec<String>, Option<String>) {
        let mut p = SseParser::default();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.feed(c).unwrap());
        }
        (out, p.last_id)
    }

    #[test]
    fn sse_joins_data_lines_and_skips_comments_and_other_fields() {
        let (out, id) = parse_all(&[b": hello\n\nevent: change\nid: 7\nretry: 10\ndata: {\"seq\":1,\ndata:\"kind\":\"x\"}\n\n", b"data: two\r\n\r", b"\ndata: three\r\r"]);
        assert_eq!(out, vec!["{\"seq\":1,\n\"kind\":\"x\"}", "two", "three"]);
        assert_eq!(id.as_deref(), Some("7"));
    }

    #[test]
    fn sse_survives_frames_split_anywhere() {
        let text = b"data: {\"seq\":5}\n\n:c\n\ndata: x\n\n";
        for cut in 0..text.len() {
            let (out, _) = parse_all(&[&text[..cut], &text[cut..]]);
            assert_eq!(out, vec!["{\"seq\":5}", "x"], "cut at {cut}");
        }
    }

    #[test]
    fn http_reader_decodes_chunked_split_anywhere() {
        let resp = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-WorkState-Api: 1\r\n\r\n5\r\nhello\r\n3;x=y\r\n ab\r\n0\r\n\r\n";
        for cut in 0..resp.len() {
            let mut r = HttpReader::default();
            let (mut a, h1) = r.feed(&resp[..cut]).unwrap();
            let (b, h2) = r.feed(&resp[cut..]).unwrap();
            a.extend(b);
            assert_eq!(String::from_utf8(a).unwrap(), "hello ab", "cut at {cut}");
            assert!(h1 ^ h2);
            assert!(r.done());
        }
    }

    #[test]
    fn http_reader_refuses_a_non_200() {
        let mut r = HttpReader::default();
        assert_eq!(r.feed(b"HTTP/1.1 401 Unauthorized\r\n\r\n").unwrap_err(), "WorkState answered HTTP 401");
    }

    #[test]
    fn http_reader_wants_read_api_v1() {
        let refused = |head: &[u8]| HttpReader::default().feed(head).unwrap_err();
        assert!(refused(b"HTTP/1.1 200 OK\r\n\r\n").contains("without X-WorkState-Api"));
        assert!(refused(b"HTTP/1.1 200 OK\r\nX-WorkState-Api: 2\r\n\r\n").contains("read API 2"));
        assert!(HttpReader::default().feed(b"HTTP/1.1 200 OK\r\nx-workstate-api: 1\r\n\r\n").unwrap().1);
    }

    #[test]
    fn an_endless_line_or_event_is_refused() {
        let mut p = SseParser::default();
        let chunk = vec![b'x'; 8 * 1024];
        let refused = (0..9).map(|_| p.feed(&chunk)).find_map(Result::err);
        assert_eq!(refused.as_deref(), Some("an event over 64 KiB"));
        let mut p = SseParser::default();
        let line = format!("data: {}\n", "y".repeat(1024));
        let refused = (0..70).map(|_| p.feed(line.as_bytes())).find_map(Result::err);
        assert!(refused.is_some(), "data lines that never end an event");
        let mut p = SseParser::default();
        assert_eq!(p.feed(format!("data: {}\n\n", "z".repeat(60 * 1024)).as_bytes()).unwrap().len(), 1);
    }
}
