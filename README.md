# RustWolf

> [!WARNING]
> **RustWolf is experimental and really unstable. It is not meant for daily use yet.**
> It is built on Servo, a young web engine: many sites break or don't render fully, video doesn't play, and pages can crash.
> It has not been security-audited, and Servo's page sandbox doesn't exist on Windows yet.
> For banking, email, shopping and anything important, use an established browser such as Firefox or LibreWolf.

A privacy-first web browser **written in Rust**, built on **[Servo](https://servo.org)**, the web engine written in Rust.

![RustWolf](screenshots/guardian.png)

## Download
Get **RustWolf-<version>-win64.exe** from the [latest release](../../releases/latest) and run it (Windows 10/11, no install needed; the exe isn't code-signed, so SmartScreen may ask you to click **More info → Run anyway**). Each release also has `SHA256SUMS.txt` to verify the download, and the full source code.
Releases are built by [GitHub Actions](.github/workflows/build.yml) for every version tag, plus a weekly build that pulls in Servo and dependency (security) fixes.

## First-run setup
The first time you open RustWolf it asks how you want to be protected (change it anytime with the gear button):
- **Search engine:** DuckDuckGo, Brave Search, Mojeek, Qwant or Kagi. No Google, and none of them sell your data.
- **Protection:** Recommended or Strict, or pick each one: ads & trackers, cookie banners, annoyances (newsletter pop-ups, chat bubbles), social widgets, malware & phishing shield, fingerprint protection, tracking-link cleaning, HTTPS-only.
- **Privacy tools:**
  - **Connect through Tor (on by default):** hides your IP address and location from every site and opens `.onion` sites. RustWolf downloads the official Tor Expert Bundle from torproject.org, refuses it unless it matches the Tor Project's published SHA-256, runs it in the background and sends every request (including DNS lookups) through it, with a separate Tor route per website. Tor stops when RustWolf closes. It hides who you are on the network, but RustWolf is not as anonymous as Tor Browser.
  - **Private front-ends:** Reddit → Redlib, YouTube → Invidious. These are free community services and can be slow or offline.

## Privacy
- **Ad, tracker and cookie-banner blocking** with Brave's Rust ad-block engine ([`adblock`](https://crates.io/crates/adblock)) and EasyList, EasyPrivacy and Fanboy's cookie list (downloaded on first start, refreshed weekly), plus RustWolf's own list. Empty ad boxes are hidden too (cosmetic filtering). Per-site shield switch with a live "blocked" counter and the list of blocked domains. Add your own lists in `%APPDATA%\RustWolf\filters.txt`.
- **Malware & phishing shield:** known dangerous sites (URLhaus and phishing lists) are stopped before any connection is made, with a warning page.
- **Fingerprint resistance:** every RustWolf reports the same common browser identity, language (en-US), CPU count, memory and platform; the screen is reported as the window size; canvas read-backs get invisible per-session noise.
- **HTTPS-only**, enforced in Servo's network layer (local addresses excepted).
- **Clean links:** `utm_*`, `fbclid`, `gclid`, `msclkid` and other tracking parameters are removed from every page you open.
- **Global Privacy Control** and Do Not Track signals.
- **Off entirely:** WebRTC (no IP leaks), WebGL, WebGPU, location, notifications, Bluetooth, gamepads, VR, wake lock. Permission requests are denied automatically.
- **Private by default:** cookies, cache and history live in memory only; closing RustWolf forgets everything.
- **No telemetry** of any kind.

## Bitwarden autofill
RustWolf can't run browser extensions (Servo doesn't support them), so it talks to **Bitwarden's official command-line tool** instead:
1. Install it once: `winget install Bitwarden.CLI`, then log in once with `bw login`.
2. In RustWolf, click the **key** button next to the shield, unlock with your master password, then click **Fill** on a saved login.

- A login is only offered when its saved address is on the same site as the page, and RustWolf checks again at the moment it fills (protects against phishing pages).
- Your master password and session key are only passed to Bitwarden's tool in memory (never on a command line), are never saved, and closing RustWolf or pressing **Lock vault** locks the vault again.

## Anti-ClickFix
"ClickFix" scams show a fake CAPTCHA or error and secretly copy a malicious command, then tell you to press Win+R and paste. In RustWolf every clipboard write from a page goes through RustWolf's own Rust code first (Servo asks the browser), so pages can't get around it: anything that looks like a command (PowerShell, cmd, mshta, rundll32, `curl … | sh`, encoded payloads, "I am not a robot" decoys…) is blocked and a red warning explains the scam. Your own Ctrl+C is never blocked.

## Safety & speed
- **Hardened mode** (part of Strict): the JavaScript JIT compilers and WebAssembly are off, closing the most-attacked part of the engine.
- **Per-site "block third-party scripts"** switch in the shield panel (scripts written into the page itself still run).
- **Letterboxing:** with fingerprint protection on, the page size is rounded to 100 px steps so your window size can't identify you.
- **Update check:** RustWolf shows an Update button when a newer release exists; the check (and filter-list downloads) go through Tor when Tor is on.
- **Verifiable releases:** every release has SHA-256 sums and signed build provenance (`gh attestation verify RustWolf-<version>-win64.exe --repo anyrustslop/rustwolf`), and every build runs a RustSec security audit of all dependencies.
- **Each page runs in its own process**, so a bad page can't reach the browser.
- **Memory guard:** a page process using more than 2 GB (less on PCs with under 12 GB of RAM) is stopped within a quarter of a second (only that page); Windows caps every RustWolf process too, so one heavy site can never freeze your PC.
- RustWolf runs below normal priority, so your other apps always stay responsive. Background tabs are throttled.

## Features
Tabs (middle-click to close), address bar that also searches with DuckDuckGo, back / forward / reload / home, loading bar, page titles and favicons. Shortcuts: Ctrl+T, Ctrl+W, Ctrl+L / F6, Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+1–9, Ctrl+R / F5, Alt+Left / Right. Original red-wolf logo drawn in code ([src/logo.rs](src/logo.rs)).

## Limits (Servo is still young)
- No video/audio playback yet on Windows, no DRM (Netflix, Spotify Web, Disney+), no browser extensions.
- Some complex sites render incorrectly (YouTube's video grid, for example); dropdowns and page dialogs are basic.
- Third-party cookies aren't separated per site yet (they are all forgotten when RustWolf closes).
- Servo's page-process sandbox doesn't exist on Windows yet; pages are isolated in separate processes but not locked down like in Firefox.

## Build
`cargo build --release` (Rust 1.88+, MSVC + Windows SDK). Servo's JavaScript engine (SpiderMonkey) is downloaded prebuilt by the `mozjs_sys` crate.
`patches/servo-net` is Servo's network crate with one change: a 256 KB HTTP/2 header limit (instead of 16 KB) so YouTube and other Google sites load.

## License
MPL-2.0. `src/keyutils.rs` and `patches/servo-net` come from Servo (MPL-2.0).
