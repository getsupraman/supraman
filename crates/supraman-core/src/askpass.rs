//! GitMan's answer to ssh's prompts: a remote call's git gets `SSH_ASKPASS` pointing at `sm askpass`, which hands
//! the prompt to the [`Bridge`] the app started for that call over the loopback, with the call's token. Only ssh's
//! prompts (a key's passphrase, a password, an unknown host key) are answered; git's own `Username for`/`Password
//! for` prompts are refused, so the user's credential helper stays the only source of https credentials.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::Duration;

use crate::secret::SecretStore;

/// The bridge of the call a `sm askpass` runs under: `<loopback address> <token>`.
pub const VAR: &str = "SUPRAMAN_ASKPASS";
/// How long a prompt waits for the user's answer.
pub const ANSWER_WAIT: Duration = Duration::from_secs(600);
const IO_WAIT: Duration = Duration::from_secs(10);
/// A prompt's text is a line or a few; anything longer is not ssh's.
const PROMPT_MAX: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// `key` is the key file when ssh names it.
    Passphrase { key: Option<String> },
    /// ssh's password authentication: `user@host`.
    Password { who: String },
    /// ssh met a host it does not know: its whole message, answered yes or no.
    HostKey { text: String },
}

/// What `text` asks, when ssh asks it; `None` for every other prompt (git's own for https).
pub fn classify(text: &str) -> Option<Prompt> {
    let trimmed = text.trim();
    if trimmed.contains("continue connecting (yes/no") {
        return Some(Prompt::HostKey { text: trimmed.to_string() });
    }
    if let Some(rest) = trimmed.strip_prefix("Enter passphrase for key ") {
        return Some(Prompt::Passphrase { key: Some(unquoted(rest.trim_end_matches(':')).to_string()) });
    }
    if let Some(rest) = trimmed.strip_prefix("Enter passphrase for ") {
        return Some(Prompt::Passphrase { key: Some(unquoted(rest.trim_end_matches(':')).to_string()) });
    }
    if trimmed == "Enter passphrase:" {
        return Some(Prompt::Passphrase { key: None });
    }
    trimmed.strip_suffix("'s password:").filter(|who| !who.contains(' ')).map(|who| Prompt::Password { who: who.to_string() })
}

fn unquoted(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).or_else(|| s.strip_prefix('"').and_then(|s| s.strip_suffix('"'))).unwrap_or(s)
}

/// Where a remembered passphrase of `key` is kept.
pub fn secret_key(key: &str) -> String {
    format!("ssh-passphrase:{}", crate::paths::path_key(Path::new(key)))
}

/// The user's answer; `remember` keeps a key's passphrase in the secret store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    pub remember: bool,
}

/// A prompt waiting for the user.
pub struct Asked {
    pub prompt: Prompt,
    reply: mpsc::Sender<Option<Reply>>,
}

impl Asked {
    /// `None` cancels: ssh gets no answer and fails.
    pub fn answer(self, reply: Option<Reply>) {
        let _ = self.reply.send(reply);
    }
}

type OnAsk = Arc<dyn Fn(Asked) + Send + Sync>;

/// One call's listener on the loopback: it answers only a client holding its token, while it lives.
pub struct Bridge {
    addr: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
}

impl Bridge {
    /// `on_ask` hears each prompt off the UI thread and answers it; a key's passphrase kept in `store` is answered
    /// from there once per call (a second prompt means it was wrong, and the user is asked).
    pub fn start(store: Option<Arc<dyn SecretStore>>, on_ask: impl Fn(Asked) + Send + Sync + 'static) -> std::io::Result<Bridge> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
        let token = crate::hex::encode(&bytes);
        let stop = Arc::new(AtomicBool::new(false));
        let serving = Serving { token: token.clone(), store, on_ask: Arc::new(on_ask), tried: Arc::default() };
        let stopped = stop.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(conn) = conn else { continue };
                let serving = serving.clone();
                thread::spawn(move || serving.serve(conn));
            }
        });
        Ok(Bridge { addr, token, stop })
    }

    /// The environment of a git whose ssh asks through `program` (`sm.exe askpass`); `DISPLAY` only where unset.
    pub fn env(&self, program: &Path) -> Vec<(String, OsString)> {
        let mut env = vec![
            ("SSH_ASKPASS".to_string(), program.as_os_str().to_owned()),
            ("SSH_ASKPASS_REQUIRE".to_string(), "force".into()),
            (VAR.to_string(), self.spec().into()),
        ];
        if std::env::var_os("DISPLAY").is_none() {
            env.push(("DISPLAY".to_string(), "1".into()));
        }
        env
    }

    /// [`VAR`]'s value.
    pub fn spec(&self) -> String {
        format!("{} {}", self.addr, self.token)
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, IO_WAIT);
    }
}

#[derive(Clone)]
struct Serving {
    token: String,
    store: Option<Arc<dyn SecretStore>>,
    on_ask: OnAsk,
    /// The keys answered from the store in this call.
    tried: Arc<Mutex<Vec<String>>>,
}

impl Serving {
    fn serve(&self, mut conn: TcpStream) {
        let _ = conn.set_read_timeout(Some(IO_WAIT));
        let mut request = Vec::new();
        if (&mut conn).take(PROMPT_MAX).read_to_end(&mut request).is_err() {
            return;
        }
        let request = String::from_utf8_lossy(&request);
        let Some((token, text)) = request.split_once('\n') else { return };
        if !same(token.trim().as_bytes(), self.token.as_bytes()) {
            return;
        }
        let answer = classify(text).and_then(|prompt| self.answer(prompt));
        let _ = conn.write_all(answer.map_or_else(|| b"0\n".to_vec(), |a| format!("1\n{a}").into_bytes()).as_slice());
        let _ = conn.shutdown(Shutdown::Write);
    }

    fn answer(&self, prompt: Prompt) -> Option<String> {
        let key = match &prompt {
            Prompt::Passphrase { key: Some(key) } => Some(secret_key(key)),
            _ => None,
        };
        if let (Some(key), Some(store)) = (&key, &self.store) {
            let mut tried = self.tried.lock().unwrap_or_else(PoisonError::into_inner);
            if !tried.contains(key) {
                tried.push(key.clone());
                if let Ok(Some(kept)) = store.get(key) {
                    return Some(kept);
                }
            }
        }
        let (reply, answered) = mpsc::channel();
        (self.on_ask)(Asked { prompt, reply });
        let reply = answered.recv_timeout(ANSWER_WAIT).ok().flatten()?;
        if let (true, Some(key), Some(store)) = (reply.remember, &key, &self.store) {
            let _ = store.set(key, &reply.text);
        }
        Some(reply.text)
    }
}

/// Compares every byte, so a wrong token takes as long whatever its prefix.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `sm askpass`'s side: the answer to `text` from the bridge [`VAR`] names; `None` when the prompt is not ssh's,
/// no bridge is set, or the user gave no answer.
pub fn ask(text: &str) -> Option<String> {
    let spec = std::env::var(VAR).ok()?;
    ask_at(&spec, text)
}

/// [`ask`] of the bridge `spec` names.
pub fn ask_at(spec: &str, text: &str) -> Option<String> {
    classify(text)?;
    let (addr, token) = spec.split_once(' ')?;
    let addr: SocketAddr = addr.parse().ok().filter(|a: &SocketAddr| a.ip().is_loopback())?;
    let mut conn = TcpStream::connect_timeout(&addr, IO_WAIT).ok()?;
    conn.write_all(format!("{token}\n{text}").as_bytes()).ok()?;
    conn.shutdown(Shutdown::Write).ok()?;
    conn.set_read_timeout(Some(ANSWER_WAIT + IO_WAIT)).ok()?;
    let mut answer = String::new();
    conn.read_to_string(&mut answer).ok()?;
    answer.strip_prefix("1\n").map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::MemoryStore;

    #[test]
    fn only_sshs_prompts_are_its_own() {
        assert_eq!(classify("Enter passphrase for key 'C:/Users/u/.ssh/id_ed25519': "), Some(Prompt::Passphrase { key: Some("C:/Users/u/.ssh/id_ed25519".into()) }));
        assert_eq!(classify("Enter passphrase: "), Some(Prompt::Passphrase { key: None }));
        assert_eq!(classify("git@host's password: "), Some(Prompt::Password { who: "git@host".into() }));
        let host = "The authenticity of host 'h (1.2.3.4)' can't be established.\nED25519 key fingerprint is SHA256:x.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert!(matches!(classify(host), Some(Prompt::HostKey { .. })));
        assert_eq!(classify("Username for 'https://github.com': "), None, "git's https prompt stays the credential helper's");
        assert_eq!(classify("Password for 'https://u@github.com': "), None);
        assert_eq!(classify("anything else"), None);
    }

    fn answering(reply: Option<Reply>) -> (impl Fn(Asked) + Send + Sync + 'static, Arc<Mutex<Vec<Prompt>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let log = heard.clone();
        (move |asked: Asked| {
            log.lock().unwrap().push(asked.prompt.clone());
            asked.answer(reply.clone());
        }, heard)
    }

    #[test]
    fn the_bridge_answers_a_holder_of_its_token_and_nothing_else() {
        let (on_ask, heard) = answering(Some(Reply { text: "secret".into(), remember: false }));
        let bridge = Bridge::start(None, on_ask).unwrap();
        assert_eq!(ask_at(&bridge.spec(), "Enter passphrase for key 'k': "), Some("secret".into()));
        assert_eq!(ask_at(&bridge.spec(), "Password for 'https://u@h': "), None, "git's own prompt is refused before it reaches the app");
        let wrong = format!("{} {}", bridge.addr, "0".repeat(32));
        assert_eq!(ask_at(&wrong, "Enter passphrase for key 'k': "), None, "a wrong token is not answered");
        assert_eq!(heard.lock().unwrap().len(), 1);
        let env = bridge.env(Path::new("C:/sm.exe"));
        assert!(env.iter().any(|(k, v)| k == "SSH_ASKPASS_REQUIRE" && v == "force"));
        assert!(!env.iter().any(|(k, _)| k == "GIT_ASKPASS"), "git's own prompts are not ours");
    }

    #[test]
    fn a_passphrase_is_kept_only_when_asked_and_answers_once_per_call() {
        let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
        let prompt = "Enter passphrase for key 'C:/k': ";
        let (on_ask, _) = answering(Some(Reply { text: "typed".into(), remember: false }));
        let bridge = Bridge::start(Some(store.clone()), on_ask).unwrap();
        assert_eq!(ask_at(&bridge.spec(), prompt), Some("typed".into()));
        assert_eq!(store.get(&secret_key("C:/k")).unwrap(), None, "nothing is written unless asked");
        drop(bridge);

        let (on_ask, heard) = answering(Some(Reply { text: "kept".into(), remember: true }));
        let bridge = Bridge::start(Some(store.clone()), on_ask).unwrap();
        assert_eq!(ask_at(&bridge.spec(), prompt), Some("kept".into()));
        assert_eq!(store.get(&secret_key("C:/k")).unwrap().as_deref(), Some("kept"));
        drop(bridge);

        let (on_ask, heard_next) = answering(Some(Reply { text: "again".into(), remember: false }));
        let bridge = Bridge::start(Some(store.clone()), on_ask).unwrap();
        assert_eq!(ask_at(&bridge.spec(), prompt), Some("kept".into()), "the kept passphrase answers first");
        assert_eq!(ask_at(&bridge.spec(), prompt), Some("again".into()), "asked again in the call, it was wrong: the user answers");
        assert_eq!((heard.lock().unwrap().len(), heard_next.lock().unwrap().len()), (1, 1));
    }

    #[test]
    fn a_cancelled_prompt_answers_nothing() {
        let (on_ask, _) = answering(None);
        let bridge = Bridge::start(None, on_ask).unwrap();
        assert_eq!(ask_at(&bridge.spec(), "Enter passphrase: "), None);
    }
}
