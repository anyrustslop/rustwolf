//! Bitwarden autofill, through Bitwarden's official command-line tool (`bw`).
//!
//! Safety rules:
//! - The master password and the session key are passed to `bw` through environment variables,
//!   never as command-line arguments (which other programs on the PC could read).
//! - The session key lives only in memory; closing RustWolf or pressing Lock locks the vault.
//! - A login is offered only when its saved address is on the same site as the page, and the
//!   check is repeated at the moment of filling.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};

use winit::event_loop::EventLoopProxy;

const NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Checking,
    NoCli,
    LoggedOut,
    Locked,
    Unlocked,
}

#[derive(Clone)]
pub struct Login {
    pub name: String,
    pub username: String,
    pub password: String,
    /// The site this login was matched for (registrable domain).
    pub site: String,
}

enum Msg {
    Status(State, Option<String>),
    Unlocked(Result<String, String>),
    Matches(String, Vec<Login>),
    Error(String),
}

pub struct Vault {
    bw: Option<PathBuf>,
    session: Option<String>,
    pub state: State,
    pub email: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
    /// Site the current matches belong to, and the matches.
    pub site: String,
    pub matches: Vec<Login>,
    fake: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    proxy: EventLoopProxy<()>,
}

/// Where the Bitwarden CLI may be installed.
fn find_bw() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            candidates.push(d.join("bw.exe"));
        }
    }
    if let Some(d) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        candidates.push(d.join("Microsoft").join("WinGet").join("Links").join("bw.exe"));
        candidates.push(d.join("RustWolf").join("bw.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join("bw.exe"));
        }
    }
    if let Some(d) = std::env::var_os("APPDATA").map(PathBuf::from) {
        candidates.push(d.join("npm").join("bw.cmd"));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Runs `bw` with plain, safe arguments; secrets only ever go through `envs`.
fn run_bw(bw: &PathBuf, args: &[&str], envs: &[(&str, &str)]) -> Result<String, String> {
    let is_cmd = bw.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd"));
    let mut cmd = if is_cmd {
        let mut c = Command::new("cmd.exe");
        c.arg("/D").arg("/C").arg(bw);
        c
    } else {
        Command::new(bw)
    };
    cmd.args(args).creation_flags(NO_WINDOW).env("BW_NOINTERACTION", "true");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| format!("Couldn't start Bitwarden CLI: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { "Bitwarden CLI reported an error.".into() } else { err })
    }
}

/// "https://login.example.com/path?x" -> "https://login.example.com" (only safe characters).
fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    if !matches!(u.scheme(), "http" | "https") || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return None;
    }
    Some(format!("{}://{}", u.scheme(), host))
}

fn parse_logins(json: &str, site: &str) -> Vec<Login> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    let Some(items) = v.as_array() else { return vec![] };
    let mut out = Vec::new();
    for item in items {
        if item["type"].as_i64() != Some(1) {
            continue; // logins only
        }
        let login = &item["login"];
        // Only offer it if one of its saved addresses is on this exact site.
        let same_site = login["uris"].as_array().is_some_and(|uris| {
            uris.iter().filter_map(|u| u["uri"].as_str()).any(|u| {
                let h = crate::host_of(u);
                !h.is_empty() && crate::base_domain(&h) == site
            })
        });
        if !same_site {
            continue;
        }
        out.push(Login {
            name: item["name"].as_str().unwrap_or("").to_string(),
            username: login["username"].as_str().unwrap_or("").to_string(),
            password: login["password"].as_str().unwrap_or("").to_string(),
            site: site.to_string(),
        });
    }
    out
}

impl Vault {
    pub fn new(proxy: EventLoopProxy<()>) -> Self {
        let (tx, rx) = channel();
        let fake = std::env::var_os("RUSTWOLF_FAKE_VAULT").is_some();
        let mut v = Self {
            bw: if fake { None } else { find_bw() },
            session: None,
            state: State::Checking,
            email: None,
            busy: false,
            error: None,
            site: String::new(),
            matches: Vec::new(),
            fake,
            tx,
            rx,
            proxy,
        };
        v.check_status();
        v
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, proxy) = (self.tx.clone(), self.proxy.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f());
            let _ = proxy.send_event(());
        });
    }

    /// Re-detects the CLI and asks it whether we're logged in / locked.
    pub fn check_status(&mut self) {
        self.error = None;
        if self.fake {
            self.state = State::Locked;
            self.email = Some("test@example.com".into());
            return;
        }
        self.bw = find_bw();
        let Some(bw) = self.bw.clone() else {
            self.state = State::NoCli;
            return;
        };
        if self.session.is_some() {
            return;
        }
        self.state = State::Checking;
        self.busy = true;
        self.spawn(move || match run_bw(&bw, &["status"], &[]) {
            Ok(out) => {
                let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
                let email = v["userEmail"].as_str().map(String::from);
                let state = match v["status"].as_str() {
                    Some("unauthenticated") => State::LoggedOut,
                    _ => State::Locked,
                };
                Msg::Status(state, email)
            }
            Err(e) => Msg::Error(e),
        });
    }

    /// Unlocks with the master password the user typed. The password string is moved into the
    /// worker and dropped there; it is passed to `bw` only through an environment variable.
    pub fn unlock(&mut self, password: String) {
        if password.is_empty() || self.busy {
            return;
        }
        self.error = None;
        self.busy = true;
        if self.fake {
            self.spawn(move || {
                if password == "test" {
                    Msg::Unlocked(Ok("fake-session".into()))
                } else {
                    Msg::Unlocked(Err("Wrong master password.".into()))
                }
            });
            return;
        }
        let Some(bw) = self.bw.clone() else { return };
        self.spawn(move || {
            let r = run_bw(&bw, &["unlock", "--passwordenv", "RUSTWOLF_BW_PASSWORD", "--raw"], &[("RUSTWOLF_BW_PASSWORD", &password)]);
            drop(password);
            match r {
                Ok(session) if !session.is_empty() => {
                    let _ = run_bw(&bw, &["sync"], &[("BW_SESSION", &session)]);
                    Msg::Unlocked(Ok(session))
                }
                Ok(_) => Msg::Unlocked(Err("Bitwarden didn't return a session.".into())),
                Err(e) => Msg::Unlocked(Err(if e.contains("Invalid master password") { "Wrong master password.".into() } else { e })),
            }
        });
    }

    pub fn lock(&mut self) {
        self.session = None;
        self.matches.clear();
        self.site.clear();
        if self.state == State::Unlocked {
            self.state = State::Locked;
        }
        if let Some(bw) = self.bw.clone() {
            std::thread::spawn(move || {
                let _ = run_bw(&bw, &["lock"], &[]);
            });
        }
    }

    /// Looks up logins for the page (when unlocked and the site changed).
    pub fn page_changed(&mut self, url: &str) {
        let host = crate::host_of(url);
        let site = if host.is_empty() { String::new() } else { crate::base_domain(&host) };
        if site == self.site {
            return;
        }
        self.site = site.clone();
        self.matches.clear();
        let (Some(session), Some(origin)) = (self.session.clone(), origin_of(url)) else { return };
        if site.is_empty() {
            return;
        }
        if self.fake {
            let s = site.clone();
            self.spawn(move || {
                let json = r#"[{"type":1,"name":"Test login","login":{"username":"wolf@example.com","password":"Correct-Horse-42","uris":[{"uri":"http://127.0.0.1/login"}]}},{"type":1,"name":"Other site","login":{"username":"x","password":"y","uris":[{"uri":"https://evil.example.net"}]}}]"#;
                Msg::Matches(s.clone(), parse_logins(json, &s))
            });
            return;
        }
        let Some(bw) = self.bw.clone() else { return };
        self.spawn(move || match run_bw(&bw, &["list", "items", "--url", &origin], &[("BW_SESSION", &session)]) {
            Ok(json) => Msg::Matches(site.clone(), parse_logins(&json, &site)),
            Err(e) => Msg::Error(e),
        });
    }

    /// Applies results from the background workers. Returns true when something changed.
    pub fn poll(&mut self, current_url: &str) -> bool {
        let mut changed = false;
        while let Ok(m) = self.rx.try_recv() {
            changed = true;
            match m {
                Msg::Status(s, email) => {
                    self.busy = false;
                    self.state = s;
                    self.email = email;
                }
                Msg::Unlocked(Ok(session)) => {
                    self.busy = false;
                    self.session = Some(session);
                    self.state = State::Unlocked;
                    self.site.clear();
                    let url = current_url.to_string();
                    self.page_changed(&url);
                }
                Msg::Unlocked(Err(e)) => {
                    self.busy = false;
                    self.error = Some(e);
                }
                Msg::Matches(site, logins) => {
                    if site == self.site {
                        self.matches = logins;
                    }
                }
                Msg::Error(e) => {
                    self.busy = false;
                    self.error = Some(e);
                }
            }
        }
        changed
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        if self.session.is_some() {
            if let Some(bw) = &self.bw {
                let _ = run_bw(bw, &["lock"], &[]);
            }
        }
    }
}

/// JavaScript that fills the visible username/password fields of the page.
pub fn fill_script(username: &str, password: &str) -> String {
    format!(
        r#"(()=>{{const u={u},p={p};
const vis=e=>e&&e.offsetParent!==null&&!e.disabled&&!e.readOnly;
const set=(e,v)=>{{e.focus();e.value=v;e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));}};
const inputs=[...document.querySelectorAll('input')];
const pw=inputs.find(e=>(e.type||'').toLowerCase()==='password'&&vis(e));
let user=null;
if(pw){{for(let k=inputs.indexOf(pw)-1;k>=0;k--){{const t=(inputs[k].type||'text').toLowerCase();if(['text','email','tel'].includes(t)&&vis(inputs[k])){{user=inputs[k];break;}}}}}}
if(!user)user=inputs.find(e=>vis(e)&&(['email'].includes((e.type||'').toLowerCase())||/user|email|login|account/i.test((e.name||'')+' '+(e.id||'')+' '+(e.autocomplete||''))));
if(user&&u)set(user,u);
if(pw&&p)set(pw,p);
return (user?'user ':'')+(pw?'password':'');}})()"#,
        u = crate::js_string(username),
        p = crate::js_string(password)
    )
}
