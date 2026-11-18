RustWolf is a privacy-first web browser **written in Rust**, built on the **Servo** web engine (also Rust).

## Download
1. Download **`RustWolf-VERSION-win64.zip`** below.
2. Unzip it anywhere and run **`RustWolf.exe`** (Windows 10/11, no install, no admin rights).
3. Optional: check the download against **`SHA256SUMS.txt`** (in PowerShell: `Get-FileHash RustWolf-VERSION-win64.zip`).

The full source code is attached as **Source code (zip / tar.gz)**.

## Privacy
- Ad, tracker and cookie-banner blocking (EasyList, EasyPrivacy, Fanboy's cookie list + RustWolf's own list) with Brave's Rust ad-block engine, including hiding empty ad boxes. Per-site shield switch with a live counter.
- Malware & phishing shield: known dangerous sites are stopped before any connection is made.
- Fingerprint resistance: uniform browser identity, language, CPU count, memory and screen size; canvas read-backs get invisible noise.
- HTTPS-only, tracking-link cleaning (`utm_*`, `fbclid`, `gclid`…), Global Privacy Control.
- WebRTC, WebGL, WebGPU, location, notifications, Bluetooth and more are switched off; permission requests are denied.
- Private by default: cookies, cache and history stay in memory and are forgotten when RustWolf closes. No telemetry.

## Safety
- Every page runs in its own process.
- Memory guard: a page using more than 2 GB is stopped within a quarter of a second, so one heavy site can't freeze your PC.
- RustWolf runs at below-normal priority so your other apps stay responsive.

## Known limits
- No video/audio playback on Windows yet, no DRM (Netflix, Spotify Web…), no extensions.
- Some complex sites don't render fully (for example YouTube's video grid).
- Servo is young: use Firefox or LibreWolf for banking and important logins.

Built automatically by GitHub Actions from commit COMMIT.
