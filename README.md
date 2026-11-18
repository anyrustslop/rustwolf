# RustWolf

> [!WARNING]
> **RustWolf is experimental and really unstable. It is not meant for daily use yet.**
> It is built on Servo, a young web engine: many sites break or don't render fully, video doesn't play, and pages can crash.
> It has not been security-audited, and Servo's page sandbox doesn't exist on Windows yet.
> For banking, email, shopping and anything important, use an established browser such as Firefox or LibreWolf.

A privacy-first web browser **written in Rust**, built on **[Servo](https://servo.org)**, the web engine written in Rust.

![RustWolf](screenshots/guardian.png)

## Download
Get **RustWolf-<version>-win64.zip** from the [latest release](../../releases/latest), unzip it and run `RustWolf.exe` (Windows 10/11, no install needed). Each release also has `SHA256SUMS.txt` to verify the download, and the full source code.
Releases are built by [GitHub Actions](.github/workflows/build.yml) for every version tag, plus a weekly build that pulls in Servo and dependency (security) fixes.

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

## Safety & speed
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
