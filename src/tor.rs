//! "Connect through Tor": downloads the official Tor Expert Bundle from torproject.org (checked
//! against the Tor Project's published SHA-256 sums), runs it in the background and lets Servo
//! send every request through Tor's HTTP tunnel port (hostnames are resolved by Tor too).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use winit::event_loop::EventLoopProxy;

/// Local port of Tor's HTTP CONNECT tunnel (only reachable from this PC).
pub const PORT: u16 = 9787;
pub fn proxy_uri() -> String {
    format!("http://127.0.0.1:{PORT}")
}

const NO_WINDOW: u32 = 0x0800_0000;
/// `progress` value meaning "failed".
pub const FAILED: u8 = 255;

pub struct Tor {
    /// 0–100 = bootstrap progress, FAILED = error (see `status`).
    pub progress: Arc<AtomicU8>,
    pub status: Arc<Mutex<String>>,
    child: Arc<Mutex<Option<Child>>>,
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("RustWolf").join("tor"))
}

fn tor_exe(base: &Path) -> PathBuf {
    base.join("tor").join("tor.exe")
}

fn get(url: &str) -> Result<ureq::Response, String> {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .get(url)
        .call()
        .map_err(|e| format!("Couldn't reach torproject.org: {e}"))
}

/// Downloads and verifies the official Tor Expert Bundle into `base`.
fn install(base: &Path, status: &Mutex<String>) -> Result<(), String> {
    *status.lock().unwrap() = "Downloading Tor from torproject.org…".into();
    let mut body = String::new();
    get("https://aus1.torproject.org/torbrowser/update_3/release/downloads.json")?
        .into_reader()
        .take(1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|e| e.to_string())?;
    let info: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let v = info["version"].as_str().ok_or("No Tor version found")?.to_string();
    if !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err("Unexpected Tor version string".into());
    }
    let file = format!("tor-expert-bundle-windows-x86_64-{v}.tar.gz");
    let mut sums = String::new();
    get(&format!("https://dist.torproject.org/torbrowser/{v}/sha256sums-signed-build.txt"))?
        .into_reader()
        .read_to_string(&mut sums)
        .map_err(|e| e.to_string())?;
    let want = sums
        .lines()
        .find(|l| l.trim_end().ends_with(&file))
        .and_then(|l| l.split_whitespace().next())
        .ok_or("Tor checksum not published")?
        .to_lowercase();

    let mut data = Vec::new();
    get(&format!("https://dist.torproject.org/torbrowser/{v}/{file}"))?
        .into_reader()
        .take(200 * 1024 * 1024)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        return Err("The Tor download didn't match the Tor Project's checksum, so it wasn't used.".into());
    }
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let archive = base.join(&file);
    std::fs::File::create(&archive).and_then(|mut f| f.write_all(&data)).map_err(|e| e.to_string())?;
    let ok = Command::new("tar.exe")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(base)
        .creation_flags(NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_file(&archive);
    if !ok || !tor_exe(base).exists() {
        return Err("Couldn't unpack Tor".into());
    }
    Ok(())
}

impl Tor {
    pub fn start(proxy: EventLoopProxy<()>) -> Self {
        let progress = Arc::new(AtomicU8::new(0));
        let status = Arc::new(Mutex::new("Starting Tor…".to_string()));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let (p, s, c) = (progress.clone(), status.clone(), child.clone());
        std::thread::Builder::new()
            .name("tor".into())
            .spawn(move || {
                let fail = |msg: String| {
                    *s.lock().unwrap() = msg;
                    p.store(FAILED, Ordering::Relaxed);
                    let _ = proxy.send_event(());
                };
                let Some(base) = dir() else { return fail("No app data folder".into()) };
                if !tor_exe(&base).exists() {
                    let _ = proxy.send_event(());
                    if let Err(e) = install(&base, &s) {
                        return fail(e);
                    }
                }
                // Our own empty torrc, so a torrc elsewhere on the PC can't change these settings.
                let torrc = base.join("rustwolf-torrc");
                let _ = std::fs::write(&torrc, "");
                let data = base.join("data");
                let geo = base.join("data").join("tor");
                let mut cmd = Command::new(tor_exe(&base));
                cmd.arg("-f")
                    .arg(&torrc)
                    .args(["--SocksPort", "0", "--HTTPTunnelPort", &format!("127.0.0.1:{PORT} IsolateDestAddr"), "--ClientOnly", "1", "--Log", "notice stdout"])
                    // Tor quits by itself if RustWolf ever closes without stopping it (crash, force-close).
                    .args(["__OwningControllerProcess", &std::process::id().to_string()])
                    .arg("--DataDirectory")
                    .arg(data.join("state"))
                    .arg("--GeoIPFile")
                    .arg(geo.join("geoip"))
                    .arg("--GeoIPv6File")
                    .arg(geo.join("geoip6"))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .creation_flags(NO_WINDOW);
                let mut ch = match cmd.spawn() {
                    Ok(ch) => ch,
                    Err(e) => return fail(format!("Couldn't start Tor: {e}")),
                };
                let out = ch.stdout.take();
                *c.lock().unwrap() = Some(ch);
                *s.lock().unwrap() = "Connecting to the Tor network…".into();
                let _ = proxy.send_event(());
                if let Some(out) = out {
                    for line in BufReader::new(out).lines().map_while(Result::ok) {
                        if let Some(i) = line.find("Bootstrapped ") {
                            let pct: String = line[i + 13..].chars().take_while(|ch| ch.is_ascii_digit()).collect();
                            if let Ok(n) = pct.parse::<u8>() {
                                p.store(n.min(100), Ordering::Relaxed);
                                *s.lock().unwrap() = if n >= 100 { "Connected to Tor".into() } else { format!("Connecting to Tor… {n}%") };
                                let _ = proxy.send_event(());
                            }
                        } else if line.contains("[err]") {
                            *s.lock().unwrap() = line.clone();
                        }
                    }
                    // stdout closed: Tor exited.
                    if p.load(Ordering::Relaxed) != 100 {
                        fail("Tor stopped unexpectedly.".into());
                    } else {
                        fail("Tor stopped. Restart RustWolf to reconnect.".into());
                    }
                }
            })
            .ok();
        Self { progress, status, child }
    }

    pub fn ready(&self) -> bool {
        self.progress.load(Ordering::Relaxed) == 100
    }

    pub fn failed(&self) -> bool {
        self.progress.load(Ordering::Relaxed) == FAILED
    }

    pub fn status(&self) -> String {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Tor {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            if let Some(ch) = c.as_mut() {
                let _ = ch.kill();
            }
        }
    }
}
