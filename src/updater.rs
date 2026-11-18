//! Automatic updates.
//!
//! 1. On start, RustWolf asks GitHub for the latest release (through Tor when Tor is on).
//! 2. If it's newer, the new exe is downloaded in the background and checked against the
//!    release's SHA256SUMS.txt. Anything that doesn't match is deleted.
//! 3. The verified exe waits next to RustWolf as `RustWolf.update.exe`. "Restart to update"
//!    (or simply the next start) swaps it in: the running exe is renamed to `RustWolf.old.exe`
//!    (Windows allows renaming a running exe), the new one takes its place and is started.
//!    The old copy is deleted on the following start.

use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU8;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use winit::event_loop::EventLoopProxy;

const REPO: &str = "anyrustslop/rustwolf";
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

/// What the toolbar shows.
#[derive(Clone, PartialEq)]
pub enum Update {
    Downloading(String),
    /// Verified and waiting: restart to install.
    Ready(String),
    /// Newer version exists but couldn't be prepared automatically: open its release page.
    Manual(String, String),
}

fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}
fn sibling(name: &str) -> Option<PathBuf> {
    exe().and_then(|e| e.parent().map(|d| d.join(name)))
}
fn staged() -> Option<PathBuf> {
    sibling("RustWolf.update.exe")
}
/// Holds the SHA-256 of the staged exe; written only after the download was verified.
fn staged_ok() -> Option<PathBuf> {
    sibling("RustWolf.update.sha256")
}
fn old() -> Option<PathBuf> {
    sibling("RustWolf.old.exe")
}

fn sha256_file(p: &Path) -> Option<String> {
    let data = std::fs::read(p).ok()?;
    Some(Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect())
}

fn version_key(s: &str) -> Vec<u64> {
    s.trim_start_matches('v').split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).take(3).collect()
}

/// Is there a verified update waiting?
pub fn staged_ready() -> bool {
    let (Some(s), Some(ok)) = (staged(), staged_ok()) else { return false };
    match (std::fs::read_to_string(ok), sha256_file(&s)) {
        (Ok(want), Some(got)) => want.trim() == got,
        _ => false,
    }
}

/// Swaps the verified update in and starts it. Returns true if the new version was started
/// (the caller should then exit).
pub fn install_and_relaunch(args: &[String]) -> bool {
    if !staged_ready() {
        return false;
    }
    let (Some(cur), Some(new), Some(old_p), Some(ok)) = (exe(), staged(), old(), staged_ok()) else { return false };
    let _ = std::fs::remove_file(&old_p);
    if std::fs::rename(&cur, &old_p).is_err() {
        return false; // folder not writable: keep running this version
    }
    if std::fs::rename(&new, &cur).is_err() {
        let _ = std::fs::rename(&old_p, &cur); // put the old one back
        return false;
    }
    let _ = std::fs::remove_file(&ok);
    let spawn = |flags: u32| std::process::Command::new(&cur).args(args).creation_flags(flags).spawn().is_ok();
    if spawn(CREATE_BREAKAWAY_FROM_JOB) || spawn(0) {
        return true;
    }
    // Couldn't start the new version: roll back.
    let _ = std::fs::rename(&cur, &new);
    let _ = std::fs::rename(&old_p, &cur);
    false
}

/// Removes the previous version left behind by the last update (once it has exited).
pub fn cleanup_old() {
    if let Some(p) = old() {
        let _ = std::fs::remove_file(p);
    }
}

fn agent(proxy: Option<String>) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(600)).user_agent("RustWolf-updater");
    if let Some(p) = proxy.and_then(|p| ureq::Proxy::new(p).ok()) {
        b = b.proxy(p);
    }
    b.build()
}

fn read_limited(resp: ureq::Response, limit: u64) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    resp.into_reader().take(limit).read_to_end(&mut v).ok()?;
    Some(v)
}

/// Background check + download. `via` returns the proxy to use (Tor) once it's ready.
pub fn start(
    tx: Sender<Update>,
    proxy: EventLoopProxy<()>,
    tor: Option<Arc<AtomicU8>>,
    via: fn(&Option<Arc<AtomicU8>>) -> Option<String>,
) {
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let send = |u: Update| {
                let _ = tx.send(u);
                let _ = proxy.send_event(());
            };
            // GitHub's normal release pages, not its API: the API has a low limit for anonymous
            // requests, which shared Tor exit addresses hit easily.
            let proxy_uri = via(&tor);
            let no_redirect = {
                let mut b = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(60)).user_agent("RustWolf-updater").redirects(0);
                if let Some(p) = proxy_uri.clone().and_then(|p| ureq::Proxy::new(p).ok()) {
                    b = b.proxy(p);
                }
                b.build()
            };
            let a = agent(proxy_uri);
            let location = match no_redirect.get(&format!("https://github.com/{REPO}/releases/latest")).call() {
                Ok(r) => r.header("location").map(String::from),
                Err(ureq::Error::Status(_, r)) => r.header("location").map(String::from),
                Err(_) => None,
            };
            let Some(location) = location else { return };
            let tag_prefix = format!("https://github.com/{REPO}/releases/tag/");
            let Some(tag) = location.strip_prefix(&tag_prefix).map(|t| t.trim_end_matches('/').to_string()) else { return };
            if !tag.starts_with('v') || !tag[1..].chars().all(|c| c.is_ascii_digit() || c == '.') {
                return;
            }
            if version_key(&tag) <= version_key(env!("CARGO_PKG_VERSION")) {
                return;
            }
            let manual = Update::Manual(tag.clone(), location.clone());
            if staged_ready() {
                return send(Update::Ready(tag));
            }
            let version = tag.trim_start_matches('v');
            let exe_name = format!("RustWolf-{version}-win64.exe");
            let exe_url = format!("https://github.com/{REPO}/releases/download/{tag}/{exe_name}");
            let sums_url = format!("https://github.com/{REPO}/releases/download/{tag}/SHA256SUMS.txt");
            send(Update::Downloading(tag.clone()));
            let sums = a.get(&sums_url).call().ok().and_then(|r| read_limited(r, 64 * 1024)).map(|b| String::from_utf8_lossy(&b).to_string());
            let want = sums.as_deref().and_then(|s| {
                s.lines().find(|l| l.split_whitespace().nth(1) == Some(exe_name.as_str())).and_then(|l| l.split_whitespace().next()).map(|h| h.to_lowercase())
            });
            let Some(want) = want else { return send(manual) };
            let Some(data) = a.get(&exe_url).call().ok().and_then(|r| read_limited(r, 400 * 1024 * 1024)) else { return send(manual) };
            let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
            if got != want {
                return send(manual); // corrupted or tampered: never used
            }
            let (Some(st), Some(ok)) = (staged(), staged_ok()) else { return send(manual) };
            if std::fs::write(&st, &data).is_err() || std::fs::write(&ok, &got).is_err() {
                let _ = std::fs::remove_file(&st);
                return send(manual); // RustWolf's folder isn't writable
            }
            send(Update::Ready(tag));
        })
        .ok();
}
