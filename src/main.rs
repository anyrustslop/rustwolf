//! RustWolf: a privacy-first web browser written in Rust, on the Servo engine (also Rust).
//!
//! - Engine: Servo (`servo` crate). Every page renders into an offscreen GL surface.
//! - UI: egui, drawn on top of the same window (tabs, address bar, shield panel).
//! - Privacy: built-in ad/tracker blocking (Brave's Rust ad-block engine), HTTPS upgrade,
//!   permission prompts denied by default, and nothing written to disk (private by default).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod blocker;
mod keyutils;
#[allow(dead_code)]
mod logo;
mod ui;

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
}

/// State Servo's callbacks can reach (they run on the main thread, during `spin_event_loop`).
pub struct Shared {
    me: OnceCell<Weak<Shared>>,
    window: Rc<Window>,
    pub events: RefCell<Vec<Ev>>,
    pub blocker: blocker::Blocker,
    /// Sites where the user turned the shield off.
    pub shields_off: RefCell<HashSet<String>>,
    pub rendering_context: Rc<OffscreenRenderingContext>,
    pub scale: Cell<f32>,
}

pub fn host_of(url: &str) -> String {
    Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_default()
}

/// Rough "same site" check (example.com vs cdn.example.com).
fn base_domain(host: &str) -> String {
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
    fn notify_closed(&self, webview: WebView) {
        self.push(Ev::Closed(webview.id()));
    }
    fn request_create_new(&self, _parent: WebView, request: servo::CreateNewWebViewRequest) {
        let webview = request
            .builder(self.rendering_context.clone())
            .hidpi_scale_factor(Scale::new(self.scale.get()))
            .delegate(self.delegate())
            .build();
        self.push(Ev::NewTab(webview));
    }
    fn request_permission(&self, _webview: WebView, request: servo::PermissionRequest) {
        // Camera, microphone, location, notifications…: denied unless the user asks for them.
        request.deny();
    }
    fn load_web_resource(&self, webview: WebView, load: servo::WebResourceLoad) {
        let req = load.request();
        let url = req.url.as_str().to_string();
        let page = webview.url().map(|u| u.to_string()).unwrap_or_default();
        let page_host = host_of(&page);
        if self.shields_off.borrow().contains(&page_host) {
            return;
        }
        // HTTPS upgrade for pages (not for local addresses).
        if req.is_for_main_frame && url.starts_with("http://") {
            let host = host_of(&url);
            let local = host == "localhost" || host.starts_with("127.") || host.ends_with(".local") || host.starts_with("192.168.");
            if !local {
                let https = url.replacen("http://", "https://", 1);
                if let Ok(target) = Url::parse(&https) {
                    webview.load(target);
                    return;
                }
            }
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
        if self.blocker.should_block(&url, &page, kind) {
            let response = servo::WebResourceResponse::new(req.url.clone().into_url()).status_code(http::StatusCode::FORBIDDEN);
            load.intercept(response).finish();
            self.push(Ev::Blocked(webview.id(), host));
        }
    }
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
}

impl Tab {
    fn blank() -> Self {
        Self { webview: None, title: "New tab".into(), url: String::new(), loading: false, blocked: Default::default(), favicon: None, status: None }
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
    FocusAddress,
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

        let servo = ServoBuilder::default().event_loop_waker(Box::new(Waker(proxy))).build();
        let shared = Rc::new(Shared {
            me: OnceCell::new(),
            window: window.clone(),
            events: RefCell::new(Vec::new()),
            blocker: blocker::Blocker::new(),
            shields_off: RefCell::new(HashSet::new()),
            rendering_context: page_rc.clone(),
            scale: Cell::new(window.scale_factor() as f32),
        });
        let _ = shared.me.set(Rc::downgrade(&shared));

        page_rc.make_current().expect("make current");
        let egui = EguiGlow::new(event_loop, page_rc.glow_gl_api(), None, None, false);
        ui::setup(&egui.egui_ctx);

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
        };
        b.ui.focus_address = true;
        if let Some(arg) = std::env::args().nth(1) {
            b.navigate(&arg);
        }
        b
    }

    fn page_rc(&self) -> &Rc<OffscreenRenderingContext> {
        &self.shared.rendering_context
    }

    fn create_webview(&self, url: Url) -> WebView {
        WebViewBuilder::new(&self.servo, self.page_rc().clone())
            .url(url)
            .hidpi_scale_factor(Scale::new(self.shared.scale.get()))
            .delegate(self.shared.delegate())
            .build()
    }

    pub fn active_webview(&self) -> Option<WebView> {
        self.tabs.get(self.active).and_then(|t| t.webview.clone())
    }

    fn navigate(&mut self, input: &str) {
        let Some(url) = ui::resolve_input(input) else { return };
        let tab = &mut self.tabs[self.active];
        tab.blocked = Default::default();
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
                self.ui.address.clear();
                self.ui.focus_address = true;
                self.page_focused = false;
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
                    }
                }
                Ev::Load(id, status) => {
                    if let Some(i) = self.tab_index(id) {
                        let t = &mut self.tabs[i];
                        t.loading = !matches!(status, LoadStatus::Complete);
                        if matches!(status, LoadStatus::Started) {
                            t.favicon = None;
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
        let page_rc = self.page_rc().clone();
        let scale = self.window.scale_factor() as f32;
        let mut actions = Vec::new();
        let mut page_rect_px = Rect::zero();
        let active = self.active_webview();

        // Build the UI. The page itself is painted by a GL callback into the area the UI leaves free.
        {
            let Browser { egui, ui, tabs, active: active_i, shared, window, .. } = self;
            let ctx_scale = scale;
            egui.run(window, |ctx| {
                let page = ui::draw(ctx, ui, tabs, *active_i, &shared.blocker, &shared.shields_off.borrow(), &mut actions);
                page_rect_px = Rect::new(
                    Point2D::new(page.min.x * ctx_scale, page.min.y * ctx_scale),
                    Size2D::new(page.width() * ctx_scale, page.height() * ctx_scale),
                );
                if let Some(w) = &active {
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
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

fn main() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let event_loop = EventLoop::with_user_event().build().expect("event loop");
    let mut app = App::Starting(event_loop.create_proxy());
    let _ = event_loop.run_app(&mut app);
    // Private by default: Servo keeps cookies, cache and history in memory only, so closing
    // RustWolf leaves nothing behind.
}
