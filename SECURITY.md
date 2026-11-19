# RustWolf security report

Self-audit of RustWolf 0.8.3 on Windows 11 (November 2026). This is an internal audit by the
project itself, **not** an independent audit. RustWolf is experimental: do not rely on it for
anything important.

## How it was tested

1. **Sandbox attack suite** (`RUSTWOLF_SANDBOX_TEST=1`): a page process that has been taken over
   is simulated by running attack code inside a real sandboxed page process
   (`guard::sandbox_attack_report`), while it renders a real site.
2. **Fingerprint bypass page**: the usual tricks to get real values past spoofing (fresh iframe,
   borrowing native functions from an iframe, web workers, `Function.prototype.toString` lie
   detection).
3. **Tracking tests**: a "tracker" site sets cookies/storage first-party, then is embedded in
   another site; what it can still read is logged. A control run with protection off confirms
   the tests detect leaks.
4. **Code review** of RustWolf's patches to Servo, ipc-channel and num_cpus, looking at what a
   hacked page process could make the browser do.
5. **ClickFix bypass strings** as unit tests (`cargo test clickfix`).

## Results: sandbox (a page process that hacked the engine)

| Attack | Before audit | 0.8.3 |
|---|---|---|
| Read / list your files, documents, browser data | blocked | blocked |
| Write files anywhere (profile, TEMP, Public) | blocked | blocked |
| Read / write your registry (e.g. autostart malware) | blocked | blocked |
| Start programs (cmd, PowerShell) | blocked | blocked |
| Read the browser's memory, take its handles | blocked | blocked |
| **Kill the browser** | **possible** | blocked |
| Read / inject into / kill other programs (Explorer) | blocked | blocked |
| **Screenshot your screen** | **possible** | blocked |
| **Keylogger on the desktop you type on** | **possible** | blocked |
| **See your clipboard** | **possible** | blocked |
| Send messages to the browser window | blocked | blocked |
| **Make the browser fetch handles from any other program (IPC confused deputy, sandbox escape)** | **possible (code review)** | blocked |
| Open its own internet connections (bypassing Tor) | possible | **still possible** |

How the sandbox works: each page process re-starts itself with a restricted Windows token (your
account "deny only", Low integrity, no privileges), in a job that forbids child processes and UI
access, on a separate, unnamed window station and desktop (own screen, clipboard and keyboard),
with exploit mitigations on. The browser and its sandbox launchers lock their own process
permissions so page processes can't open them. Servo's IPC is patched so page processes can talk
to the browser without being able to reach into it, and the browser only takes handles from
RustWolf's own page processes.

### Still open
- **Network:** a hacked page process could open its own connections and so skip Tor. Blocking
  that needs an AppContainer sandbox (planned) or an administrator firewall rule.
- **Page processes may pass engine handles to each other** (Servo's IPC needs it). One hacked
  page process could interfere with another; it can't read or kill it.
- **Denial of service:** a hacked page process can crash the browser by sending malformed IPC.
- **Folders with unusual permissions:** a page process runs as a restricted copy of your account,
  so a folder that grants access to *all users* (`BUILTIN\Users`) stays readable. Windows doesn't
  do that by default. To check a folder: `icacls <folder>`.

## Results: fingerprinting

| Trick | Before audit | 0.8.3 |
|---|---|---|
| Real CPU count via a fresh iframe / borrowed getter / worker | leaked (real count) | 4 everywhere (set in the engine) |
| Real timezone via iframe / worker | leaked (real zone) | UTC+0 everywhere (set in the engine) |
| Plugins, screen, pixel ratio via a fresh iframe | leaked | spoofed in iframes too |
| Spotting spoofed functions with `toString()` | visible | look native |
| Language via `Intl` | leaked (fixed in 0.8.2) | en-US (engine) |

### Still open
- Workers report the zone name `UTC` (Tor Browser: `Atlantic/Reykjavik`), and timers in workers
  aren't coarsened.
- Deep engine differences (CSS/layout quirks, missing APIs, rendering) can still tell Servo apart
  from Firefox. RustWolf matches Tor Browser on the values sites usually check, not on everything.

## Results: tracking

- Third-party cookies (HTTP and `document.cookie`) and web storage of embedded sites: blocked /
  partitioned; tested with a control run.
- Still open: script cookies of an embedded site are only blocked while that site isn't open in
  one of your tabs; `BroadcastChannel` isn't partitioned.

## Results: anti-ClickFix

Bypasses that worked before and are caught now: caret / quote / backtick obfuscation
(`p^ower^shell`, `pow""ershell`), WebDAV / network paths (`\\host@SSL\DavWWWRoot\x.exe`), and more
Windows tools that run downloaded code (`wmic`, `cmstp`, `msbuild`, `finger user@host`, ...).
Normal text still copies (tested).

## Ratings

The project's own judgment, out of 10, for Windows as of November 2026. Higher is better.

| | Hides your IP | Tracker blocking | Anti-fingerprinting | Exploit protection (sandbox, hardening) | Maturity, audits, patch speed | Site compatibility |
|---|---|---|---|---|---|---|
| **RustWolf 0.8.3** | 8 (Tor by default) | 8 | 6 | 5 | 2 | 2 |
| Tor Browser | 10 | 8 | 9 | 7 | 9 | 6 |
| Mullvad Browser | 5 (with Mullvad VPN: 8) | 8 | 9 | 7 | 8 | 8 |
| LibreWolf | 1 | 8 | 6 | 7 | 7 | 9 |
| Brave | 2 (Tor windows: 6) | 8 | 7 | 9 | 9 | 10 |
| Firefox (default) | 1 | 6 | 3 | 7 | 10 | 9 |
| Chrome | 1 | 2 | 2 | 10 | 10 | 10 |

**Bottom line:** RustWolf is now a reasonably private browser for reading (Tor by default, blocked
trackers, Tor-Browser-like fingerprint, a sandbox that survived this audit), but it is young,
unaudited and breaks many sites. For anonymity use Tor Browser; for everyday private browsing use
Mullvad Browser, LibreWolf or Brave.

## Reporting a vulnerability

Please open an issue (or a private security advisory) on
[GitHub](https://github.com/anyrustslop/rustwolf/security/advisories/new).
