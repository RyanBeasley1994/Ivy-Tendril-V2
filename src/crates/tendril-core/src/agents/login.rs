//! Signing an agent's CLI back in from the UI.
//!
//! The daemon usually runs on another machine, so "run `claude auth login`" is advice the operator
//! cannot follow from the app. Both CLIs that matter here can sign in without a terminal: Claude
//! prints an authorize URL and reads the code its callback page shows back on stdin, and Codex's
//! device flow prints a URL and a one-time code and polls on its own. So the daemon runs the login,
//! the app shows the URL, and for Claude relays the pasted code.
//!
//! One session per agent: starting again replaces the last one. A session that is never finished is
//! killed after [`SESSION_TTL`], so an abandoned dialog does not leave a login process behind.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};

use crate::agents::resolution::normalize_agent_name;
use crate::jobs::process_tree::{kill_tree, DEFAULT_KILL_GRACE};

const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
/// How long `start` waits for the CLI to print its URL.
const URL_WAIT: Duration = Duration::from_secs(20);
/// How long `submit_code` waits for the CLI to finish exchanging the code.
const EXCHANGE_WAIT: Duration = Duration::from_secs(45);
const MAX_OUTPUT: usize = 16 * 1024;

static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https://[^\s<>]+").expect("url regex"));
static ANSI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)").expect("ansi regex"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginState {
    Running,
    Succeeded,
    Failed,
}

/// What the sign-in dialog shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginView {
    pub agent: String,
    pub state: LoginState,
    /// The page to open in the operator's own browser.
    pub url: Option<String>,
    /// Whether the CLI is waiting for a code pasted back (Claude), rather than polling (Codex).
    pub needs_code: bool,
    /// What the CLI printed, escapes removed: Codex's one-time code is in here.
    pub output: String,
}

struct Session {
    pid: u32,
    needs_code: bool,
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    output: Arc<Mutex<String>>,
    exit: Arc<Mutex<Option<i32>>>,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> = LazyLock::new(Default::default);

/// The login command for `agent`, and whether it reads a code from stdin.
fn login_command(agent: &str) -> Option<(&'static str, &'static [&'static str], bool)> {
    match agent {
        "claude" => Some(("claude", &["auth", "login"], true)),
        "codex" => Some(("codex", &["login", "--device-auth"], false)),
        _ => None,
    }
}

/// Whether the UI can sign `agent` in itself.
pub fn supports_login(agent: &str) -> bool {
    login_command(&normalize_agent_name(agent)).is_some()
}

fn clean(text: &str) -> String {
    ANSI.replace_all(text, "").replace('\r', "")
}

fn view(agent: &str, session: &Session) -> LoginView {
    let output = clean(&session.output.lock().expect("login output"));
    let state = match *session.exit.lock().expect("login exit") {
        None => LoginState::Running,
        Some(0) => LoginState::Succeeded,
        Some(_) => LoginState::Failed,
    };
    LoginView {
        agent: agent.to_string(),
        state,
        url: URL.find(&output).map(|m| m.as_str().trim_end_matches(['.', ',', ')']).to_string()),
        needs_code: session.needs_code,
        output: output.trim().to_string(),
    }
}

fn spawn_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(mut pipe: R, sink: Arc<Mutex<String>>) {
    tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf).await {
            if n == 0 {
                break;
            }
            let mut out = sink.lock().expect("login output");
            if out.len() < MAX_OUTPUT {
                out.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    });
}

/// Starts `agent`'s login and waits for it to print the URL to open.
pub async fn start(agent: &str) -> Result<LoginView, String> {
    let agent = normalize_agent_name(agent);
    let (binary, args, needs_code) =
        login_command(&agent).ok_or_else(|| format!("Signing in to '{agent}' from the app is not supported"))?;
    cancel(&agent);

    let mut cmd = Command::new(binary);
    cmd.args(args)
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.as_std_mut().process_group(0);
    }
    let mut child = cmd.spawn().map_err(|err| format!("Could not start `{binary}`: {err}"))?;
    let pid = child.id().unwrap_or(0);

    let output = Arc::new(Mutex::new(String::new()));
    if let Some(out) = child.stdout.take() {
        spawn_reader(out, output.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_reader(err, output.clone());
    }
    let exit = Arc::new(Mutex::new(None));
    let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));

    {
        let exit = exit.clone();
        tokio::spawn(async move {
            let code = match tokio::time::timeout(SESSION_TTL, child.wait()).await {
                Ok(Ok(status)) => status.code().unwrap_or(-1),
                _ => {
                    if pid != 0 {
                        let _ = tokio::task::spawn_blocking(move || kill_tree(pid, DEFAULT_KILL_GRACE)).await;
                    }
                    let _ = child.kill().await;
                    -1
                }
            };
            *exit.lock().expect("login exit") = Some(code);
        });
    }

    let session = Session { pid, needs_code, stdin, output, exit };
    let deadline = tokio::time::Instant::now() + URL_WAIT;
    loop {
        let current = view(&agent, &session);
        if current.url.is_some() || current.state != LoginState::Running || tokio::time::Instant::now() >= deadline {
            SESSIONS.lock().expect("login sessions").insert(agent.clone(), session);
            return Ok(current);
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Where `agent`'s login has got to, or `None` when there is no session.
pub fn status(agent: &str) -> Option<LoginView> {
    let agent = normalize_agent_name(agent);
    let sessions = SESSIONS.lock().expect("login sessions");
    sessions.get(&agent).map(|session| view(&agent, session))
}

/// Hands Claude the code its callback page showed, and waits for the exchange to finish.
pub async fn submit_code(agent: &str, code: &str) -> Result<LoginView, String> {
    let agent = normalize_agent_name(agent);
    let (stdin, exit) = {
        let sessions = SESSIONS.lock().expect("login sessions");
        let session = sessions.get(&agent).ok_or("No sign-in is in progress; start again")?;
        (session.stdin.clone(), session.exit.clone())
    };
    {
        let mut guard = stdin.lock().await;
        let pipe = guard.as_mut().ok_or("This sign-in does not take a code")?;
        let line = format!("{}\n", code.trim());
        pipe.write_all(line.as_bytes()).await.map_err(|err| format!("The login process has gone: {err}"))?;
        pipe.flush().await.map_err(|err| format!("The login process has gone: {err}"))?;
    }
    let deadline = tokio::time::Instant::now() + EXCHANGE_WAIT;
    while exit.lock().expect("login exit").is_none() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    status(&agent).ok_or_else(|| "The sign-in session was cancelled".to_string())
}

/// Ends `agent`'s login, if one is running.
pub fn cancel(agent: &str) {
    let agent = normalize_agent_name(agent);
    let Some(session) = SESSIONS.lock().expect("login sessions").remove(&agent) else {
        return;
    };
    if session.exit.lock().expect("login exit").is_none() && session.pid != 0 {
        let pid = session.pid;
        std::thread::spawn(move || kill_tree(pid, DEFAULT_KILL_GRACE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_clis_with_a_headless_login_are_offered() {
        assert!(supports_login("claude"));
        assert!(supports_login("codex"));
        assert!(!supports_login("gemini"));
    }

    #[test]
    fn escapes_are_stripped_and_the_url_is_found() {
        let session = Session {
            pid: 0,
            needs_code: true,
            stdin: Arc::new(tokio::sync::Mutex::new(None)),
            output: Arc::new(Mutex::new(
                "\x1b[1mOpening browser\x1b[0m\r\nIf the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&state=x\nPaste code here if prompted >".into(),
            )),
            exit: Arc::new(Mutex::new(None)),
        };
        let v = view("claude", &session);
        assert_eq!(v.url.as_deref(), Some("https://claude.com/cai/oauth/authorize?code=true&state=x"));
        assert_eq!(v.state, LoginState::Running);
        assert!(!v.output.contains('\x1b'));
    }
}
