> [!WARNING]
> **RustWolf is experimental and really unstable. It is not meant for daily use yet.**
> It is built on Servo, a young web engine: many sites break or don't render fully, video doesn't play, and pages can crash.
> It has not been security-audited, and its page sandbox is much weaker than Firefox's or Chrome's.
> For banking, email, shopping and anything important, use an established browser such as Firefox or LibreWolf.

RustWolf is a privacy-first web browser **written in Rust**, built on the **Servo** web engine (also Rust).

## Download
1. Download **`RustWolf-VERSION-win64.exe`** below.
2. Run it (Windows 10/11, no install, no admin rights). The exe isn't code-signed, so Windows SmartScreen may warn you: click **More info → Run anyway**.
3. Optional: check the download against **`SHA256SUMS.txt`** (in PowerShell: `Get-FileHash RustWolf-VERSION-win64.exe`), or prove it was built by this repository's GitHub workflow with its signed provenance: `gh attestation verify RustWolf-VERSION-win64.exe --repo anyrustslop/rustwolf`.

The full source code is attached as **Source code (zip / tar.gz)**.

## Privacy
- Ad, tracker and cookie-banner blocking (EasyList, EasyPrivacy, Fanboy's cookie list + RustWolf's own list) with Brave's Rust ad-block engine, including hiding empty ad boxes. Per-site shield switch with a live counter.
- Malware & phishing shield: known dangerous sites are stopped before any connection is made.
- Fingerprint resistance: reports exactly what Tor Browser / Mullvad Browser report; canvas read-backs get invisible noise.
- Third-party cookies blocked; every page runs in a sandboxed process.
- HTTPS-only, tracking-link cleaning (`utm_*`, `fbclid`, `gclid`…), Global Privacy Control.
- WebRTC, WebGL, WebGPU, location, notifications, Bluetooth and more are switched off; permission requests are denied.
- Private by default: cookies, cache and history stay in memory and are forgotten when RustWolf closes. No telemetry.

## New in 0.7
- **Page sandbox:** every page process locks itself into a Windows job that forbids starting any program (a hacked page can't launch PowerShell or malware) and blocks clipboard, desktop and system-setting access, plus Windows exploit mitigations (no DLL injection, no DLLs from network shares, strict handle checks, and no executable-code creation in hardened mode).
- **Third-party cookies blocked** (on by default, new switch in setup): ads and trackers embedded in pages get no cookies and can't set any. Done in Servo's network layer.
- **Tor Browser crowd fingerprint:** RustWolf now reports exactly what Tor Browser 16 and Mullvad Browser report on Windows: Firefox 153 identity and HTTP headers, en-US, 4 cores, UTC (Atlantic/Reykjavik) timezone, touch points, Do Not Track, and Tor Browser's letterboxing steps (200 × 100 px, at most 1400 × 900).
- Limits: the sandbox still runs pages with your user rights (no AppContainer yet), and deep fingerprint tests can still tell Servo apart.

## New in 0.6
- **Automatic updates:** RustWolf downloads new versions in the background (through Tor when Tor is on), checks them against the release's SHA-256, and installs them when you click "Restart to update" or the next time you open it. The previous version is kept until the new one has started. (Coming from 0.5.x or older? Download this version once by hand; after that it updates itself.)

## New in 0.5.1
- Firefox-style window: no Windows title bar; minimize / maximize / close sit at the top right of the tab strip (drag the empty space to move, double-click to maximize, resize from the edges).
- Bitwarden button shows a Bitwarden-style shield; the Tor status shows an onion.

## New in 0.5
- **Anti-ClickFix:** pages can't put commands (PowerShell, cmd, mshta, curl | sh…) in your clipboard; RustWolf blocks them and explains the scam. Your own Ctrl+C is never blocked.
- **Hardened mode** (in Strict): JavaScript JIT and WebAssembly off, closing the most-attacked part of the engine.
- **Per-site "block third-party scripts"** switch in the shield panel.
- **Letterboxing:** page size rounded to 100 px steps so your window size can't identify you.
- **Update check** with an Update button; update checks and filter-list downloads go through Tor when Tor is on.
- **Signed build provenance** for every release, and a RustSec security audit of all dependencies on every build.

## First-run setup and Tor
- Pick your search engine (DuckDuckGo, Brave Search, Mojeek, Qwant, Kagi), your protection level (Recommended / Strict or each switch) and privacy tools on first start; change them anytime with the gear button.
- **Connect through Tor:** RustWolf downloads the official Tor Expert Bundle (checksum-verified) and routes all traffic through it.
- New blocking options: annoyances and social widgets (Fanboy lists).

## Bitwarden autofill
Click the key button next to the shield to unlock your Bitwarden vault (through Bitwarden's official CLI: `winget install Bitwarden.CLI`, then `bw login` once) and fill saved logins on matching sites.

## Safety
- Every page runs in its own process.
- Memory guard: a page using more than 2 GB is stopped within a quarter of a second, so one heavy site can't freeze your PC.
- RustWolf runs at below-normal priority so your other apps stay responsive.

## Known limits
- No video/audio playback on Windows yet, no DRM (Netflix, Spotify Web…), no extensions.
- Some complex sites don't render fully (for example YouTube's video grid).
- Servo is young: use Firefox or LibreWolf for banking and important logins.

Built automatically by GitHub Actions from commit COMMIT.
