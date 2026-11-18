//! RustWolf: a privacy-first web browser written in Rust, on the Servo engine (also Rust).
//!
//! - Engine: Servo (`servo` crate). Every page renders into an offscreen GL surface.
//! - UI: egui, drawn on top of the same window (tabs, address bar, shield panel).
//! - Privacy: built-in ad/tracker blocking (Brave's Rust ad-block engine), HTTPS upgrade,
//!   permission prompts denied by default, and nothing written to disk (private by default).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod blocker;
mod guard;
mod keyutils;
#[allow(dead_code)]
mod logo;
mod settings;
mod tor;
mod setup_ui;
mod ui;
mod vault;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use egui_glow::EguiGlow;
use euclid::{Point2D, Rect, Scale, Size2D};
use servo::{
    DevicePixel, DevicePoint, EventLoopWaker, InputEvent, LoadStatus, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent, OffscreenRenderingContext, RenderingContext, Servo,
    ServoBuilder, WebView, WebViewBuilder, WebViewDelegate, WebViewId, WheelDelta, WheelEvent, WheelMode,
    WindowRenderingContext,
};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Icon, Window};

pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("RustWolf"))
}

// ---------------------------------------------------------------- events from Servo

pub enum Ev {
    Title(WebViewId, Option<String>),
    Url(WebViewId, Url),
    Load(WebViewId, LoadStatus),
    Favicon(WebViewId),
    Status(WebViewId, Option<String>),
    NewTab(WebView),
    Blocked(WebViewId, String),
    Closed(WebViewId),
    Danger(WebViewId, Url),
    /// CSS to add to a page (queued: Servo can't start a script from inside another script's answer).
    Css(WebView, String),
    Crashed(WebViewId),
    /// A page tried to put a command in the clipboard (ClickFix scam).
    ClickFix(WebViewId, String),
}

/// State Servo's callbacks can reach (they run on the main thread, during `spin_event_loop`).
pub struct Shared {
    me: OnceCell<Weak<Shared>>,
    window: Rc<Window>,
    pub events: RefCell<Vec<Ev>>,
    pub blocker: RefCell<blocker::Blocker>,
    pub ucm: Rc<servo::UserContentManager>,
    pub settings: RefCell<settings::Settings>,
    /// The fingerprint-protection script (added/removed when the setting changes).
    pub fp_script: Rc<servo::UserScript>,
    /// When the user last pressed Ctrl+C / Ctrl+X themselves (their own copies are never blocked).
    pub user_copy: Cell<Option<std::time::Instant>>,
    /// Sites where the user turned the shield off.
    pub shields_off: RefCell<HashSet<String>>,
    pub rendering_context: Rc<OffscreenRenderingContext>,
    pub scale: Cell<f32>,
}

pub fn host_of(url: &str) -> String {
    Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_default()
}

/// Rough "same site" check (example.com vs cdn.example.com).
pub fn base_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    let n = if parts.len() >= 3 && parts[parts.len() - 2].len() <= 3 && parts[parts.len() - 1].len() == 2 { 3 } else { 2 };
    parts[parts.len().saturating_sub(n)..].join(".")
}

impl Shared {
    fn push(&self, ev: Ev) {
        self.events.borrow_mut().push(ev);
        self.window.request_redraw();
    }
    fn delegate(&self) -> Rc<dyn WebViewDelegate> {
        self.me.get().and_then(Weak::upgrade).expect("shared state alive")
    }
    fn clip(&self) -> Rc<dyn servo::ClipboardDelegate> {
        self.me.get().and_then(Weak::upgrade).expect("shared state alive")
    }
}

/// Every clipboard write from a page passes through here first (Servo asks the browser app),
/// so a page script can't get around the ClickFix check.
impl servo::ClipboardDelegate for Shared {
    fn clear(&self, _webview: WebView) {
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.clear();
        }
    }
    fn get_text(&self, _webview: WebView, request: servo::StringRequest) {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(t) => request.success(t),
            Err(e) => request.failure(e.to_string()),
        }
    }
    fn set_text(&self, webview: WebView, new_contents: String) {
        let user_copied = self.user_copy.get().is_some_and(|t| t.elapsed().as_millis() < 1500);
        if !user_copied && guard::looks_like_command(&new_contents) {
            self.push(Ev::ClickFix(webview.id(), new_contents));
            return;
        }
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.set_text(new_contents);
        }
    }
}

impl WebViewDelegate for Shared {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }
    fn notify_page_title_changed(&self, webview: WebView, title: Option<String>) {
        self.push(Ev::Title(webview.id(), title));
    }
    fn notify_url_changed(&self, webview: WebView, url: Url) {
        self.push(Ev::Url(webview.id(), url));
    }
    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        self.push(Ev::Load(webview.id(), status));
    }
    fn notify_favicon_changed(&self, webview: WebView) {
        self.push(Ev::Favicon(webview.id()));
    }
    fn notify_status_text_changed(&self, webview: WebView, status: Option<String>) {
        self.push(Ev::Status(webview.id(), status));
    }
    fn notify_crashed(&self, webview: WebView, _reason: String, _backtrace: Option<String>) {
        self.push(Ev::Crashed(webview.id()));
    }
    fn notify_closed(&self, webview: WebView) {
        self.push(Ev::Closed(webview.id()));
    }
    fn request_create_new(&self, _parent: WebView, request: servo::CreateNewWebViewRequest) {
        let webview = request
            .builder(self.rendering_context.clone())
            .user_content_manager(self.ucm.clone())
            .clipboard_delegate(self.clip())
            .hidpi_scale_factor(Scale::new(self.scale.get()))
            .delegate(self.delegate())
            .build();
        self.push(Ev::NewTab(webview));
    }
    fn request_permission(&self, _webview: WebView, request: servo::PermissionRequest) {
        // Camera, microphone, location, notifications…: denied unless the user asks for them.
        request.deny();
    }
    /// Every page navigation: malware/phishing check, tracking-link cleaning, HTTPS upgrade.
    fn request_navigation(&self, webview: WebView, request: servo::NavigationRequest) {
        let mut url = request.url.clone();
        let host = host_of(url.as_str());
        let protected = !self.shields_off.borrow().contains(&host);
        let mut changed = false;
        let s = self.settings.borrow().clone();
        if protected && s.malware && self.blocker.borrow().is_dangerous(url.as_str()) {
            request.deny();
            self.push(Ev::Danger(webview.id(), url));
            return;
        }
        if protected && s.clean_links {
            if let Some(clean) = blocker::clean_url(&url) {
                url = clean;
                changed = true;
            }
        }
        if let Some(front_end) = s.redirect(&url) {
            url = front_end;
            changed = true;
        }
        // .onion sites are end-to-end encrypted by Tor itself and usually use plain http.
        let local = host == "localhost" || host.starts_with("127.") || host.ends_with(".local") || host.starts_with("192.168.") || host.starts_with("10.") || host.ends_with(".onion");
        if s.https_only && url.scheme() == "http" && !local && url.set_scheme("https").is_ok() {
            changed = true;
        }
        if changed {
            request.deny();
            webview.load(url);
        } else {
            request.allow();
        }
    }
    fn load_web_resource(&self, webview: WebView, load: servo::WebResourceLoad) {
        let req = load.request();
        let url = req.url.as_str().to_string();
        let page = webview.url().map(|u| u.to_string()).unwrap_or_default();
        let page_host = host_of(&page);
        if !self.settings.borrow().block_ads || self.shields_off.borrow().contains(&page_host) {
            return;
        }
        if req.is_for_main_frame {
            return;
        }
        let host = host_of(&url);
        if base_domain(&host) == base_domain(&page_host) {
            return; // first-party
        }
        let destination = format!("{:?}", req.destination).to_lowercase();
        let kind = match destination.as_str() {
            "script" | "serviceworker" | "sharedworker" | "worker" => "script",
            "image" => "image",
            "style" => "stylesheet",
            "font" => "font",
            "audio" | "video" | "track" => "media",
            "iframe" | "frame" => "sub_frame",
            "" | "none" => "xmlhttprequest",
            _ => "other",
        };
        if kind == "script" && self.settings.borrow().no_3p_scripts.contains(&base_domain(&page_host)) {
            let response = servo::WebResourceResponse::new(req.url.clone().into_url()).status_code(http::StatusCode::FORBIDDEN);
            load.intercept(response).finish();
            self.push(Ev::Blocked(webview.id(), host));
            return;
        }
        if self.blocker.borrow().should_block(&url, &page, kind) {
            let response = servo::WebResourceResponse::new(req.url.clone().into_url()).status_code(http::StatusCode::FORBIDDEN);
            load.intercept(response).finish();
            self.push(Ev::Blocked(webview.id(), host));
        }
    }
}

/// Loads the filter lists for these settings in the background: first from disk (instant),
/// then downloads any that are missing or older than a week.
fn load_lists(tx: std::sync::mpsc::Sender<blocker::Blocker>, proxy: EventLoopProxy<()>, settings: settings::Settings, tor: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>) {
    std::thread::Builder::new()
        .name("filter-lists".into())
        .spawn(move || {
            let _ = tx.send(blocker::Blocker::full(false, &settings, None));
            let _ = proxy.send_event(());
            if blocker::Blocker::needs_update(&settings) {
                let via = wait_for_tor(&tor);
                let _ = tx.send(blocker::Blocker::full(true, &settings, via.as_deref()));
                let _ = proxy.send_event(());
            }
        })
        .ok();
}

/// With Tor on, network requests RustWolf makes itself (filter lists, update check) wait for Tor
/// and go through it, so they don't reveal your IP either. Returns the proxy to use.
fn wait_for_tor(tor: &Option<std::sync::Arc<std::sync::atomic::AtomicU8>>) -> Option<String> {
    let progress = tor.as_ref()?;
    for _ in 0..240 {
        match progress.load(std::sync::atomic::Ordering::Relaxed) {
            100 => return Some(tor::proxy_uri()),
            tor::FAILED => break,
            _ => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
    // Tor isn't working: better to skip the download than to leak the IP.
    Some("http://127.0.0.1:9".into())
}

/// Checks GitHub for a newer RustWolf release (through Tor when Tor is on).
fn check_for_update(tx: std::sync::mpsc::Sender<(String, String)>, proxy: EventLoopProxy<()>, tor: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>) {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(60));
            if let Some(p) = wait_for_tor(&tor).and_then(|p| ureq::Proxy::new(p).ok()) {
                builder = builder.proxy(p);
            }
            let Ok(resp) = builder.build().get("https://api.github.com/repos/anyrustslop/rustwolf/releases/latest").set("User-Agent", "RustWolf").call() else { return };
            let mut body = String::new();
            if std::io::Read::read_to_string(&mut std::io::Read::take(resp.into_reader(), 2 * 1024 * 1024), &mut body).is_err() {
                return;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { return };
            let (Some(tag), Some(url)) = (v["tag_name"].as_str(), v["html_url"].as_str()) else { return };
            let num = |s: &str| -> Vec<u64> { s.trim_start_matches('v').split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).take(3).collect() };
            if num(tag) > num(env!("CARGO_PKG_VERSION")) && url.starts_with("https://github.com/anyrustslop/rustwolf/") {
                let _ = tx.send((tag.to_string(), url.to_string()));
                let _ = proxy.send_event(());
            }
        })
        .ok();
}

/// Cosmetic filtering: hides ad boxes and banners left empty by blocked requests.
fn hide_ad_boxes(shared: Rc<Shared>, webview: WebView) {
    let Some(url) = webview.url().map(|u| u.to_string()) else { return };
    if !shared.settings.borrow().block_ads || shared.shields_off.borrow().contains(&host_of(&url)) || !url.starts_with("http") {
        return;
    }
    let (css, generichide, exceptions) = shared.blocker.borrow().cosmetic_css(&url);
    inject_css(&webview, "__rw_hide", &css);
    if generichide {
        return;
    }
    // Generic rules: ask the page which classes/ids it uses, then hide the matching ad elements.
    let w2 = webview.clone();
    webview.evaluate_javascript(
        "(()=>{const c=new Set(),i=new Set();for(const e of document.querySelectorAll('[class],[id]')){if(e.id)i.add(e.id);for(const k of e.classList)c.add(k);}return [...c].join('\\u0001')+'\\u0002'+[...i].join('\\u0001');})()",
        move |result| {
            let Ok(servo::JSValue::String(s)) = result else { return };
            let (c, i) = s.split_once('\u{2}').unwrap_or((s.as_str(), ""));
            let classes: Vec<String> = c.split('\u{1}').filter(|x| !x.is_empty()).map(String::from).collect();
            let ids: Vec<String> = i.split('\u{1}').filter(|x| !x.is_empty()).map(String::from).collect();
            let css = shared.blocker.borrow().class_id_css(&classes, &ids, &exceptions);
            if !css.is_empty() {
                shared.push(Ev::Css(w2, css));
            }
        },
    );
}

fn inject_css(w: &WebView, id: &str, css: &str) {
    if css.is_empty() {
        return;
    }
    let js = format!(
        "(()=>{{let s=document.getElementById('{id}');if(!s){{s=document.createElement('style');s.id='{id}';(document.head||document.documentElement).appendChild(s);}}s.textContent={};}})()",
        js_string(css)
    );
    w.evaluate_javascript(js, |_| {});
}

pub fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '<' => out.push_str("\\u003c"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------- tabs

pub struct Tab {
    pub webview: Option<WebView>,
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub blocked: blocker::Blocked,
    pub favicon: Option<egui::TextureHandle>,
    pub status: Option<String>,
    /// Shown instead of the page (stopped page, dangerous site).
    pub notice: Option<Notice>,
    /// Blocked clipboard command, shown in a red warning bar.
    pub alert: Option<String>,
}

pub struct Notice {
    pub title: String,
    pub text: String,
    pub url: String,
    pub danger: bool,
}

impl Tab {
    fn blank() -> Self {
        Self { webview: None, title: "New tab".into(), url: String::new(), loading: false, blocked: Default::default(), favicon: None, status: None, notice: None, alert: None }
    }
}

/// What the user asked for this frame (applied after the UI is drawn).
pub enum Action {
    Navigate(String),
    NewTab,
    CloseTab(usize),
    Select(usize),
    Back,
    Forward,
    Reload,
    Stop,
    Home,
    ToggleShield,
    VaultCheck,
    OpenSettings,
    OpenInNewTab(String),
    AlertDismiss,
    AlertCopy,
    ToggleScripts,
    Restart,
    ApplySettings(Box<settings::Settings>),
    VaultUnlock,
    VaultLock,
    VaultFill(usize),
    FocusAddress,
    /// Open a blocked dangerous page anyway (shield off for that site).
    Proceed(String),
}

// ---------------------------------------------------------------- the running browser

struct Waker(EventLoopProxy<()>);
impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(Waker(self.0.clone()))
    }
    fn wake(&self) {
        let _ = self.0.send_event(());
    }
}

pub struct Browser {
    window: Rc<Window>,
    window_rc: Rc<WindowRenderingContext>,
    shared: Rc<Shared>,
    servo: Servo,
    egui: EguiGlow,
    pub ui: ui::UiState,
    pub tabs: Vec<Tab>,
    pub active: usize,
    modifiers: ModifiersState,
    /// Mouse position relative to the page, in device pixels.
    mouse: Point2D<f32, DevicePixel>,
    mouse_window: PhysicalPosition<f64>,
    /// Page area in device pixels (inside the window).
    page_rect: Cell<Rect<f32, DevicePixel>>,
    page_focused: bool,
    guard: guard::MemoryGuard,
    lists_rx: std::sync::mpsc::Receiver<blocker::Blocker>,
    guard_cooldown: std::time::Instant,
    lists_ready: bool,
    update_rx: std::sync::mpsc::Receiver<(String, String)>,
    tor: Option<tor::Tor>,
    exit_requested: bool,
    lists_tx: std::sync::mpsc::Sender<blocker::Blocker>,
    proxy: EventLoopProxy<()>,
    vault: vault::Vault,
    /// Address given on the command line, opened once the protection lists are loaded.
    pending_url: Option<String>,
}

impl Browser {
    fn new(event_loop: &ActiveEventLoop, proxy: EventLoopProxy<()>) -> Self {
        let icon = Icon::from_rgba(logo::render_rgba(64), 64, 64).ok();
        let attrs = Window::default_attributes()
            .with_title("RustWolf")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 820.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(640.0, 420.0))
            .with_theme(Some(winit::window::Theme::Dark))
            .with_window_icon(icon);
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));
        let display = event_loop.display_handle().expect("display handle");
        let handle = window.window_handle().expect("window handle");
        let window_rc = Rc::new(WindowRenderingContext::new(display, handle, window.inner_size()).expect("GL context"));
        window_rc.make_current().expect("make current");
        let page_rc = Rc::new(window_rc.offscreen_context(window.inner_size()));

        // Keep heavy sites from taking over the PC: fewer layout threads and a cap on JavaScript memory.
        let mut prefs = servo::Preferences::default();
        prefs.layout_threads = 2;
        prefs.js_mem_max = 1536; // MB of JavaScript heap per page
        // Privacy lockdown of the engine itself.
        let user_settings = settings::Settings::load();
        prefs.network_enforce_tls_enabled = user_settings.https_only; // HTTPS-only (local addresses excepted)
        // Pages may use copy buttons; every page-made copy goes through the ClickFix guard.
        prefs.dom_async_clipboard_enabled = true;
        prefs.dom_exec_command_enabled = true;
        if user_settings.hardened {
            // No JIT compilers and no WebAssembly: the JavaScript engine only interprets.
            prefs.js_disable_jit = true;
            prefs.js_baseline_jit_enabled = false;
            prefs.js_ion_enabled = false;
            prefs.js_asmjs_enabled = false;
            prefs.js_wasm_enabled = false;
            prefs.js_wasm_baseline_enabled = false;
            prefs.js_wasm_ion_enabled = false;
            prefs.js_native_regex_enabled = false;
        }
        if user_settings.tor {
            // Every request goes through Tor's HTTP tunnel; Tor also resolves the hostnames.
            prefs.network_http_proxy_uri = tor::proxy_uri();
            prefs.network_https_proxy_uri = tor::proxy_uri();
            prefs.network_http_no_proxy = "localhost,127.0.0.1".into();
        }
        prefs.network_http_disk_cache = String::new(); // nothing cached on disk
        prefs.user_agent = guard::USER_AGENT.into(); // blend in with the most common browser
        prefs.intl_locale_override = "en-US".into();
        prefs.dom_webgl2_enabled = false;
        prefs.dom_webgpu_enabled = false;
        prefs.dom_webrtc_enabled = false; // no IP leaks through WebRTC
        prefs.dom_geolocation_enabled = false;
        prefs.dom_notification_enabled = false;
        prefs.dom_bluetooth_enabled = false;
        prefs.dom_gamepad_enabled = false;
        prefs.dom_webxr_enabled = false;
        prefs.dom_wakelock_enabled = false;
        // Every page runs in its own process: a bad or hacked page can't reach the browser,
        // and a runaway page can be stopped without touching the rest.
        let opts = servo::Opts { multiprocess: std::env::var_os("RUSTWOLF_SINGLE_PROCESS").is_none(), ..Default::default() };
        let servo = ServoBuilder::default().opts(opts).preferences(prefs).event_loop_waker(Box::new(Waker(proxy.clone()))).build();
        if std::env::var_os("RUSTWOLF_LOG").is_some() {
            servo.setup_logging();
        }
        let shared = Rc::new(Shared {
            me: OnceCell::new(),
            window: window.clone(),
            events: RefCell::new(Vec::new()),
            blocker: RefCell::new(blocker::Blocker::builtin()),
            ucm: Rc::new(servo::UserContentManager::new(&servo)),
            settings: RefCell::new(user_settings.clone()),
            user_copy: Cell::new(None),
            fp_script: {
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7);
                Rc::new(servo::UserScript::new(guard::fingerprint_script(seed), None))
            },
            shields_off: RefCell::new(HashSet::new()),
            rendering_context: page_rc.clone(),
            scale: Cell::new(window.scale_factor() as f32),
        });
        let _ = shared.me.set(Rc::downgrade(&shared));
        if user_settings.fingerprint {
            shared.ucm.add_script(shared.fp_script.clone());
        }

        page_rc.make_current().expect("make current");
        let egui = EguiGlow::new(event_loop, page_rc.glow_gl_api(), None, None, false);
        ui::setup(&egui.egui_ctx);

        // Full filter lists load in the background (first start downloads them).
        let tor = user_settings.tor.then(|| tor::Tor::start(proxy.clone()));
        let (lists_tx, lists_rx) = std::sync::mpsc::channel();
        let tor_progress = tor.as_ref().map(|t| t.progress.clone());
        load_lists(lists_tx.clone(), proxy.clone(), user_settings.clone(), tor_progress.clone());
        let (update_tx, update_rx) = std::sync::mpsc::channel();
        check_for_update(update_tx, proxy.clone(), tor_progress);
        let p3 = proxy.clone();
        let guard = guard::MemoryGuard::start(move || {
            let _ = p3.send_event(());
        });

        let mut b = Self {
            window,
            window_rc,
            shared,
            servo,
            egui,
            ui: ui::UiState::new(),
            tabs: vec![Tab::blank()],
            active: 0,
            modifiers: ModifiersState::empty(),
            mouse: Point2D::zero(),
            mouse_window: PhysicalPosition::new(0.0, 0.0),
            page_rect: Cell::new(Rect::zero()),
            page_focused: false,
            guard,
            lists_rx,
            guard_cooldown: std::time::Instant::now(),
            lists_ready: false,
            update_rx,
            tor,
            exit_requested: false,
            lists_tx,
            proxy: proxy.clone(),
            vault: vault::Vault::new(proxy.clone()),
            pending_url: None,
        };
        b.ui.focus_address = true;
        if !user_settings.setup_done {
            b.ui.setup_open = true;
            b.ui.setup_step = 0;
            b.ui.draft = user_settings.clone();
        }
        if let Some(arg) = std::env::args().nth(1) {
            b.pending_url = Some(arg);
            b.ui.focus_address = false;
        }
        b
    }

    fn page_rc(&self) -> &Rc<OffscreenRenderingContext> {
        &self.shared.rendering_context
    }

    fn create_webview(&self, url: Url) -> WebView {
        WebViewBuilder::new(&self.servo, self.page_rc().clone())
            .url(url)
            .user_content_manager(self.shared.ucm.clone())
            .clipboard_delegate(self.shared.clip())
            .hidpi_scale_factor(Scale::new(self.shared.scale.get()))
            .delegate(self.shared.delegate())
            .build()
    }

    pub fn active_webview(&self) -> Option<WebView> {
        self.tabs.get(self.active).and_then(|t| t.webview.clone())
    }

    fn navigate(&mut self, input: &str) {
        let s = self.shared.settings.borrow().clone();
        let Some(mut url) = ui::resolve_input(input, &s) else { return };
        if let Some(front_end) = s.redirect(&url) {
            url = front_end;
        }
        let protected = !self.shared.shields_off.borrow().contains(&host_of(url.as_str()));
        if protected {
            if s.clean_links {
                if let Some(clean) = blocker::clean_url(&url) {
                    url = clean;
                }
            }
            if s.malware && self.shared.blocker.borrow().is_dangerous(url.as_str()) {
                let t = &mut self.tabs[self.active];
                t.webview = None;
                t.url = url.to_string();
                t.title = "Dangerous site blocked".into();
                t.notice = Some(Notice {
                    title: "RustWolf blocked a dangerous site".into(),
                    text: format!("{} is on a list of known malware or phishing sites. It may try to steal your passwords or install harmful software.", host_of(url.as_str())),
                    url: url.to_string(),
                    danger: true,
                });
                self.ui.editing = false;
                return;
            }
        }
        let tab = &mut self.tabs[self.active];
        tab.blocked = Default::default();
        tab.notice = None;
        tab.url = url.to_string();
        match &tab.webview {
            Some(w) => w.load(url),
            None => {
                let w = self.create_webview(url);
                self.tabs[self.active].webview = Some(w);
            }
        }
        self.page_focused = true;
        self.ui.editing = false;
        self.ui.blur = true;
        self.sync_visibility();
    }

    /// Only the visible tab renders; background tabs are throttled by Servo (saves CPU and RAM).
    fn sync_visibility(&self) {
        for (i, t) in self.tabs.iter().enumerate() {
            if let Some(w) = &t.webview {
                if i == self.active {
                    w.show();
                    w.set_focused(true);
                } else {
                    w.hide();
                }
            }
        }
        self.window.request_redraw();
    }

    fn tab_index(&self, id: WebViewId) -> Option<usize> {
        self.tabs.iter().position(|t| t.webview.as_ref().is_some_and(|w| w.id() == id))
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Navigate(s) => self.navigate(&s),
            Action::NewTab => {
                self.tabs.push(Tab::blank());
                self.active = self.tabs.len() - 1;
                self.ui.focus_address = true;
                self.ui.address.clear();
                self.page_focused = false;
                self.sync_visibility();
            }
            Action::CloseTab(i) => {
                if i < self.tabs.len() {
                    self.tabs.remove(i); // dropping the WebView closes the page
                    if self.tabs.is_empty() {
                        self.tabs.push(Tab::blank());
                        self.ui.focus_address = true;
                    }
                    if self.active >= self.tabs.len() || (i < self.active) {
                        self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
                    }
                    self.ui.editing = false;
                    self.sync_visibility();
                }
            }
            Action::Select(i) => {
                if i < self.tabs.len() {
                    self.active = i;
                    self.ui.editing = false;
                    self.sync_visibility();
                }
            }
            Action::Back => {
                if let Some(w) = self.active_webview() {
                    if w.can_go_back() {
                        w.go_back(1);
                    }
                }
            }
            Action::Forward => {
                if let Some(w) = self.active_webview() {
                    if w.can_go_forward() {
                        w.go_forward(1);
                    }
                }
            }
            Action::Reload => {
                if let Some(w) = self.active_webview() {
                    self.tabs[self.active].blocked = Default::default();
                    w.reload();
                }
            }
            Action::Stop => {
                if let Some(w) = self.active_webview() {
                    let _ = w.evaluate_javascript("window.stop()", |_| {});
                }
            }
            Action::Home => {
                let t = &mut self.tabs[self.active];
                t.webview = None;
                t.title = "New tab".into();
                t.url.clear();
                t.favicon = None;
                t.blocked = Default::default();
                t.loading = false;
                t.notice = None;
                self.ui.address.clear();
                self.ui.focus_address = true;
                self.page_focused = false;
            }
            Action::VaultCheck => self.vault.check_status(),
            Action::Restart => {
                // Start a fresh RustWolf outside our memory-limit job (so it isn't closed with us), then quit.
                if let Ok(exe) = std::env::current_exe() {
                    use std::os::windows::process::CommandExt;
                    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
                    if std::process::Command::new(exe).creation_flags(CREATE_BREAKAWAY_FROM_JOB).spawn().is_ok() {
                        self.exit_requested = true;
                    }
                }
            }
            Action::OpenInNewTab(url) => {
                self.apply(Action::NewTab);
                self.navigate(&url);
            }
            Action::AlertDismiss => {
                self.tabs[self.active].alert = None;
            }
            Action::AlertCopy => {
                if let Some(text) = self.tabs[self.active].alert.take() {
                    if let Ok(mut c) = arboard::Clipboard::new() {
                        let _ = c.set_text(text);
                    }
                }
            }
            Action::ToggleScripts => {
                let site = base_domain(&host_of(&self.tabs[self.active].url));
                if !site.is_empty() {
                    let mut s = self.shared.settings.borrow().clone();
                    if let Some(i) = s.no_3p_scripts.iter().position(|x| *x == site) {
                        s.no_3p_scripts.remove(i);
                    } else {
                        s.no_3p_scripts.push(site);
                    }
                    s.save();
                    *self.shared.settings.borrow_mut() = s;
                    self.apply(Action::Reload);
                }
            }
            Action::OpenSettings => {
                self.ui.draft = self.shared.settings.borrow().clone();
                self.ui.setup_step = 1;
                self.ui.setup_open = true;
                self.ui.shield_open = false;
                self.ui.vault_open = false;
            }
            Action::ApplySettings(new) => {
                let old = self.shared.settings.borrow().clone();
                new.save();
                if new.fingerprint != old.fingerprint {
                    if new.fingerprint {
                        self.shared.ucm.add_script(self.shared.fp_script.clone());
                    } else {
                        self.shared.ucm.remove_script(self.shared.fp_script.clone());
                    }
                }
                if new.tor != old.tor || new.hardened != old.hardened {
                    self.ui.restart_needed = true;
                }
                let lists_changed = new.cookie_banners != old.cookie_banners || new.annoyances != old.annoyances || new.social != old.social;
                *self.shared.settings.borrow_mut() = (*new).clone();
                if lists_changed {
                    load_lists(self.lists_tx.clone(), self.proxy.clone(), *new, self.tor.as_ref().map(|t| t.progress.clone()));
                }
                if self.tabs[self.active].webview.is_none() {
                    self.ui.focus_address = true;
                }
            }
            Action::VaultUnlock => {
                let pw = std::mem::take(&mut self.ui.vault_password);
                self.vault.unlock(pw);
            }
            Action::VaultLock => self.vault.lock(),
            Action::VaultFill(i) => {
                // Check again, right now, that the page is on the login's site.
                let url = self.tabs[self.active].url.clone();
                let host = host_of(&url);
                if let (Some(login), Some(w)) = (self.vault.matches.get(i).cloned(), self.active_webview()) {
                    if !host.is_empty() && base_domain(&host) == login.site && url.starts_with("http") {
                        w.evaluate_javascript(vault::fill_script(&login.username, &login.password), |_| {});
                        self.page_focused = true;
                    }
                }
            }
            Action::ToggleShield => {
                let host = host_of(&self.tabs[self.active].url);
                if host.is_empty() {
                    return;
                }
                {
                    let mut off = self.shared.shields_off.borrow_mut();
                    if !off.remove(&host) {
                        off.insert(host);
                    }
                }
                self.apply(Action::Reload);
            }
            Action::Proceed(url) => {
                self.shared.shields_off.borrow_mut().insert(host_of(&url));
                self.navigate(&url);
            }
            Action::FocusAddress => {
                self.ui.focus_address = true;
                self.page_focused = false;
            }
        }
    }

    fn handle_servo_events(&mut self) {
        let events = std::mem::take(&mut *self.shared.events.borrow_mut());
        for ev in events {
            match ev {
                Ev::NewTab(w) => {
                    let url = w.url().map(|u| u.to_string()).unwrap_or_default();
                    self.tabs.push(Tab { webview: Some(w), url, loading: true, ..Tab::blank() });
                    self.active = self.tabs.len() - 1;
                    self.sync_visibility();
                }
                Ev::Title(id, title) => {
                    if let Some(i) = self.tab_index(id) {
                        let t = &mut self.tabs[i];
                        t.title = title.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| host_of(&t.url));
                    }
                }
                Ev::Url(id, url) => {
                    if let Some(i) = self.tab_index(id) {
                        let t = &mut self.tabs[i];
                        if host_of(&t.url) != host_of(url.as_str()) {
                            t.blocked = Default::default();
                        }
                        t.url = url.to_string();
                        // The page moved on: show its real address unless the user is typing a new one.
                        if i == self.active && !self.ui.typed {
                            self.ui.editing = false;
                            self.ui.blur = true;
                        }
                    }
                }
                Ev::Load(id, status) => {
                    if let Some(i) = self.tab_index(id) {
                        let t = &mut self.tabs[i];
                        t.loading = !matches!(status, LoadStatus::Complete);
                        if matches!(status, LoadStatus::Started) {
                            t.favicon = None;
                        }
                        if matches!(status, LoadStatus::HeadParsed | LoadStatus::Complete) {
                            if let Some(w) = t.webview.clone() {
                                hide_ad_boxes(self.shared.clone(), w);
                            }
                        }
                    }
                }
                Ev::Favicon(id) => {
                    if let Some(i) = self.tab_index(id) {
                        let img = self.tabs[i].webview.as_ref().and_then(|w| w.favicon().map(|f| ui::to_color_image(&f)));
                        if let Some(img) = img {
                            let tex = self.egui.egui_ctx.load_texture(format!("fav{i}-{id:?}"), img, Default::default());
                            self.tabs[i].favicon = Some(tex);
                        }
                    }
                }
                Ev::Status(id, s) => {
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].status = s;
                    }
                }
                Ev::Blocked(id, host) => {
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].blocked.add(host);
                    }
                }
                Ev::Danger(id, url) => {
                    if let Some(i) = self.tab_index(id) {
                        let t = &mut self.tabs[i];
                        t.webview = None;
                        t.loading = false;
                        t.url = url.to_string();
                        t.title = "Dangerous site blocked".into();
                        t.notice = Some(Notice {
                            title: "RustWolf blocked a dangerous site".into(),
                            text: format!("{} is on a list of known malware or phishing sites. It may try to steal your passwords or install harmful software.", host_of(url.as_str())),
                            url: url.to_string(),
                            danger: true,
                        });
                    }
                }
                Ev::Css(w, css) => inject_css(&w, "__rw_hide2", &css),
                Ev::ClickFix(id, text) => {
                    if let Some(i) = self.tab_index(id) {
                        self.tabs[i].alert = Some(text);
                        if i != self.active {
                            self.apply(Action::Select(i));
                        }
                    }
                }
                Ev::Crashed(id) => {
                    if let Some(i) = self.tab_index(id) {
                        let killed = self.guard.killed_mb.swap(0, std::sync::atomic::Ordering::Relaxed);
                        let limit = self.guard.limit_mb;
                        let t = &mut self.tabs[i];
                        t.webview = None;
                        t.loading = false;
                        t.notice = Some(if killed > 0 {
                            Notice {
                                title: "Page stopped to protect your PC".into(),
                                text: format!(
                                    "This page used {:.1} GB of memory, more than the {:.1} GB limit, so RustWolf stopped it. Your other tabs are fine.",
                                    killed as f64 / 1024.0,
                                    limit as f64 / 1024.0
                                ),
                                url: t.url.clone(),
                                danger: false,
                            }
                        } else {
                            Notice {
                                title: "This page crashed".into(),
                                text: "The page stopped working (it may have run out of memory). Your other tabs are fine.".into(),
                                url: t.url.clone(),
                                danger: false,
                            }
                        });
                    }
                }
                Ev::Closed(id) => {
                    if let Some(i) = self.tab_index(id) {
                        self.apply(Action::CloseTab(i));
                    }
                }
            }
        }
    }

    fn redraw(&mut self) {
        self.handle_servo_events();
        let url = self.tabs[self.active].url.clone();
        self.vault.page_changed(&url);
        let page_rc = self.page_rc().clone();
        let scale = self.window.scale_factor() as f32;
        let mut actions = Vec::new();
        let mut page_rect_px = Rect::zero();
        let active = self.active_webview();

        // Build the UI. The page itself is painted by a GL callback into the area the UI leaves free.
        {
            let Browser { egui, ui, tabs, active: active_i, shared, window, vault, .. } = self;
            let ctx_scale = scale;
            egui.run(window, |ctx| {
                let page = ui::draw(ctx, ui, tabs, *active_i, &shared.blocker.borrow(), &shared.shields_off.borrow(), vault, &shared.settings.borrow(), &mut actions);
                let page = if page.is_positive() && page.is_finite() { page } else { egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO) };
                page_rect_px = Rect::new(
                    Point2D::new(page.min.x * ctx_scale, page.min.y * ctx_scale),
                    Size2D::new(page.width() * ctx_scale, page.height() * ctx_scale),
                );
                if let Some(w) = active.as_ref().filter(|_| page.width() > 1.0 && page.height() > 1.0) {
                    let size = PhysicalSize::new(page_rect_px.size.width.round() as u32, page_rect_px.size.height.round() as u32);
                    if size.width > 0 && size.height > 0 && w.size() != Size2D::new(size.width as f32, size.height as f32) {
                        w.resize(size);
                    }
                    page_rc.make_current().expect("make current");
                    w.paint();
                    if let Some(render) = page_rc.render_to_parent_callback() {
                        ctx.layer_painter(egui::LayerId::background()).add(egui::PaintCallback {
                            rect: page,
                            callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                                let clip = info.viewport_in_pixels();
                                let r = Rect::new(Point2D::new(clip.left_px, clip.from_bottom_px), Size2D::new(clip.width_px, clip.height_px));
                                render(painter.gl(), r)
                            })),
                        });
                    }
                }
            });
        }
        self.page_rect.set(page_rect_px);

        page_rc.make_current().expect("make current");
        page_rc.parent_context().prepare_for_rendering();
        self.egui.paint(&self.window);
        page_rc.parent_context().present();

        for a in actions {
            self.apply(a);
        }
        // Keep the window title in sync.
        let title = format!("{} — RustWolf", self.tabs[self.active].title);
        if self.window.title() != title {
            self.window.set_title(&title);
        }
        if self.egui.egui_ctx.has_requested_repaint() || self.tabs.iter().any(|t| t.loading) {
            self.window.request_redraw();
        }
    }

    /// Picks up freshly loaded filter lists, and stops a page that eats too much memory.
    fn background_checks(&mut self) {
        let url = self.tabs[self.active].url.clone();
        if self.vault.poll(&url) {
            self.window.request_redraw();
        }
        if let Ok(update) = self.update_rx.try_recv() {
            self.ui.update = Some(update);
            self.window.request_redraw();
        }
        while let Ok(full) = self.lists_rx.try_recv() {
            *self.shared.blocker.borrow_mut() = full;
            self.lists_ready = true;
            self.window.request_redraw();
        }
        if let Some(t) = &self.tor {
            let st = (t.ready(), t.failed(), t.status());
            if self.ui.tor.as_ref() != Some(&st) {
                self.ui.tor = Some(st);
                self.window.request_redraw();
            }
        }
        let tor_settled = self.tor.as_ref().is_none_or(|t| t.ready() || t.failed());
        if self.lists_ready && tor_settled {
            if let Some(url) = self.pending_url.take() {
                self.navigate(&url);
            }
        }
        if self.guard.over() && self.guard_cooldown.elapsed().as_secs() >= 5 {
            self.guard_cooldown = std::time::Instant::now();
            // Stop the page being viewed first; if memory is still too high next time, background tabs go too.
            let order: Vec<usize> = std::iter::once(self.active).chain((0..self.tabs.len()).filter(|i| *i != self.active)).collect();
            if let Some(i) = order.into_iter().find(|i| self.tabs[*i].webview.is_some()) {
                let used = self.guard.current_mb.load(std::sync::atomic::Ordering::Relaxed);
                let t = &mut self.tabs[i];
                t.webview = None; // closing the page frees its memory
                t.loading = false;
                t.notice = Some(Notice {
                    title: "Page stopped to protect your PC".into(),
                    text: format!(
                        "This page made RustWolf use {:.1} GB of memory, more than the {:.1} GB limit, so it was stopped before your PC could slow down.",
                        used as f64 / 1024.0,
                        self.guard.limit_mb as f64 / 1024.0
                    ),
                    url: t.url.clone(),
                    danger: false,
                });
                self.window.request_redraw();
            }
        }
    }

    fn over_page(&self, p: PhysicalPosition<f64>) -> bool {
        let r = self.page_rect.get();
        let x = p.x as f32;
        let y = p.y as f32;
        x >= r.min_x() && x < r.max_x() && y >= r.min_y() && y < r.max_y() && !self.ui.popup_open
    }

    /// Browser shortcuts work whether the page or the toolbar has focus.
    fn shortcut(&mut self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{KeyCode, PhysicalKey};
        if event.state != ElementState::Pressed {
            return false;
        }
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        let alt = self.modifiers.alt_key();
        let PhysicalKey::Code(code) = event.physical_key else { return false };
        let action = match code {
            KeyCode::KeyT if ctrl => Action::NewTab,
            KeyCode::KeyW if ctrl => Action::CloseTab(self.active),
            KeyCode::KeyL if ctrl => Action::FocusAddress,
            KeyCode::F6 => Action::FocusAddress,
            KeyCode::KeyR if ctrl => Action::Reload,
            KeyCode::F5 => Action::Reload,
            KeyCode::Tab if ctrl => {
                let n = self.tabs.len();
                Action::Select(if shift { (self.active + n - 1) % n } else { (self.active + 1) % n })
            }
            KeyCode::ArrowLeft if alt => Action::Back,
            KeyCode::ArrowRight if alt => Action::Forward,
            KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8
                if ctrl =>
            {
                let i = match code {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    KeyCode::Digit6 => 5,
                    KeyCode::Digit7 => 6,
                    _ => 7,
                };
                Action::Select(i)
            }
            KeyCode::Digit9 if ctrl => Action::Select(self.tabs.len() - 1),
            _ => return false,
        };
        self.apply(action);
        self.window.request_redraw();
        true
    }

    /// Where the pointer really is (asks Windows). Some input tools click without sending
    /// movement events first; this keeps clicks landing in the right place.
    fn os_cursor_position(&self) -> Option<PhysicalPosition<f64>> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let handle = self.window.window_handle().ok()?;
        let winit::raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() else { return None };
        let mut p = POINT { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut p) == 0 || ScreenToClient(h.hwnd.get() as _, &mut p) == 0 {
                return None;
            }
        }
        Some(PhysicalPosition::new(p.x as f64, p.y as f64))
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if self.exit_requested {
            event_loop.exit();
            return;
        }
        if matches!(event, WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. }) {
            if let Some(pos) = self.os_cursor_position() {
                if pos != self.mouse_window {
                    #[allow(unused_unsafe)]
                    let device_id = unsafe { winit::event::DeviceId::dummy() };
                    self.window_event(event_loop, WindowEvent::CursorMoved { device_id, position: pos });
                }
            }
        }
        self.servo.spin_event_loop();
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return; // minimized
                }
                self.window_rc.resize(*size);
                self.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.shared.scale.set(*scale_factor as f32);
                for t in &self.tabs {
                    if let Some(w) = &t.webview {
                        w.set_hidpi_scale_factor(Scale::new(*scale_factor as f32));
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event: key, .. } => {
                if key.state == ElementState::Pressed
                    && self.modifiers.control_key()
                    && matches!(key.physical_key, winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyC | winit::keyboard::KeyCode::KeyX))
                {
                    self.shared.user_copy.set(Some(std::time::Instant::now()));
                }
                if self.shortcut(key) {
                    return;
                }
                if self.page_focused && !self.ui.editing {
                    if let Some(w) = self.active_webview() {
                        w.notify_input_event(InputEvent::Keyboard(keyutils::keyboard_event_from_winit(key, self.modifiers)));
                    }
                    return;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_window = *position;
                let r = self.page_rect.get();
                let was_inside = r.size.width > 0.0 && Rect::from_size(r.size).contains(self.mouse);
                self.mouse = Point2D::new(position.x as f32 - r.min_x(), position.y as f32 - r.min_y());
                if let Some(w) = self.active_webview() {
                    if self.over_page(*position) {
                        w.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(DevicePoint::new(self.mouse.x, self.mouse.y).into())));
                    } else if was_inside {
                        w.notify_input_event(InputEvent::MouseLeftViewport(MouseLeftViewportEvent::default()));
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if std::env::var_os("RUSTWOLF_DEBUG_INPUT").is_some() {
                    eprintln!("[input] {:?} {:?} at {:?} page_rect={:?} over_page={} page_focused={}", state, button, self.mouse_window, self.page_rect.get(), self.over_page(self.mouse_window), self.page_focused);
                }
                if *state == ElementState::Pressed && *button == MouseButton::Back {
                    self.apply(Action::Back);
                    return;
                }
                if *state == ElementState::Pressed && *button == MouseButton::Forward {
                    self.apply(Action::Forward);
                    return;
                }
                if self.over_page(self.mouse_window) {
                    if let Some(w) = self.active_webview() {
                        if *button == MouseButton::Left || *button == MouseButton::Right {
                            self.page_focused = true;
                            self.ui.editing = false;
                            self.ui.blur = true;
                        }
                        let b = match button {
                            MouseButton::Left => ServoMouseButton::Primary,
                            MouseButton::Right => ServoMouseButton::Secondary,
                            MouseButton::Middle => ServoMouseButton::Auxiliary,
                            MouseButton::Back => ServoMouseButton::Back,
                            MouseButton::Forward => ServoMouseButton::Forward,
                            MouseButton::Other(v) => ServoMouseButton::Other(*v),
                        };
                        let a = if *state == ElementState::Pressed { MouseButtonAction::Down } else { MouseButtonAction::Up };
                        w.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(a, b, DevicePoint::new(self.mouse.x, self.mouse.y).into())));
                        self.window.request_redraw();
                        return;
                    }
                } else if *state == ElementState::Pressed {
                    self.page_focused = false;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.over_page(self.mouse_window) {
                    if let Some(w) = self.active_webview() {
                        let (dx, dy) = match delta {
                            MouseScrollDelta::LineDelta(x, y) => ((*x * 38.0) as f64, (*y * 38.0) as f64),
                            MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                        };
                        w.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                            WheelDelta { x: dx, y: dy, z: 0.0, mode: WheelMode::DeltaPixel },
                            DevicePoint::new(self.mouse.x, self.mouse.y).into(),
                        )));
                        return;
                    }
                }
            }
            _ => {}
        }
        let response = self.egui.on_window_event(&self.window, &event);
        if response.repaint {
            self.window.request_redraw();
        }
    }
}

// ---------------------------------------------------------------- winit app

enum App {
    Starting(EventLoopProxy<()>),
    Running(Box<Browser>),
}

impl ApplicationHandler<()> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let App::Starting(proxy) = self {
            let b = Browser::new(event_loop, proxy.clone());
            *self = App::Running(Box::new(b));
        }
    }
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let App::Running(b) = self {
            b.servo.spin_event_loop();
            b.background_checks();
            if !b.shared.events.borrow().is_empty() {
                b.window.request_redraw();
            }
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: winit::window::WindowId, event: WindowEvent) {
        if let App::Running(b) = self {
            b.window_event(event_loop, event);
        }
    }
}

/// Runs RustWolf below normal priority, so a heavy page can never make Windows itself lag:
/// your other apps (and the mouse, the taskbar…) always get the CPU first.
fn lower_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS};
    unsafe {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--content-process" {
        // A page process started by Servo.
        lower_priority();
        servo::run_content_process(args[2].clone());
        return;
    }
    lower_priority();
    guard::hard_memory_cap();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let event_loop = EventLoop::with_user_event().build().expect("event loop");
    let mut app = App::Starting(event_loop.create_proxy());
    let _ = event_loop.run_app(&mut app);
    // Private by default: Servo keeps cookies, cache and history in memory only, so closing
    // RustWolf leaves nothing behind.
}
