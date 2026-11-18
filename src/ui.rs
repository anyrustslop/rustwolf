//! RustWolf's interface (egui): tab strip, toolbar, shield panel and the new-tab page.

use std::collections::HashSet;

use egui::{vec2, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Key, Layout, Margin, Pos2, Rect, Sense, Stroke, TextEdit};

use crate::{blocker::Blocker, host_of, Action, Tab};

pub const BG: Color32 = Color32::from_rgb(12, 12, 16);
pub const CHROME: Color32 = Color32::from_rgb(19, 19, 25);
pub const RAISED: Color32 = Color32::from_rgb(30, 30, 38);
pub const HOVER: Color32 = Color32::from_rgb(38, 38, 48);
pub const TEXT: Color32 = Color32::from_rgb(242, 242, 245);
pub const SUBTLE: Color32 = Color32::from_rgb(160, 160, 172);
pub const RED: Color32 = Color32::from_rgb(229, 56, 59);
pub const RED_HI: Color32 = Color32::from_rgb(255, 90, 95);

mod icon {
    pub const BACK: &str = "\u{E72B}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const RELOAD: &str = "\u{E72C}";
    pub const STOP: &str = "\u{E711}";
    pub const HOME: &str = "\u{E80F}";
    pub const ADD: &str = "\u{E710}";
    pub const CLOSE: &str = "\u{E8BB}";
    pub const SHIELD: &str = "\u{EA18}";
    pub const LOCK: &str = "\u{E72E}";
    pub const UNLOCK: &str = "\u{E785}";
    pub const SEARCH: &str = "\u{E721}";
    pub const GLOBE: &str = "\u{E774}";
}

pub struct UiState {
    pub address: String,
    pub editing: bool,
    pub focus_address: bool,
    pub blur: bool,
    pub shield_open: bool,
    /// First-run setup / Settings screen.
    pub setup_open: bool,
    pub setup_step: usize,
    pub draft: crate::settings::Settings,
    /// Tor connection shown in the toolbar: (ready, failed, status text).
    pub tor: Option<(bool, bool, String)>,
    pub restart_needed: bool,
    pub maximized: bool,
    /// Newer release on GitHub: (version tag, release page).
    pub update: Option<(String, String)>,
    pub vault_open: bool,
    /// Master password being typed (cleared as soon as it's handed to Bitwarden).
    pub vault_password: String,
    pub popup_open: bool,
    /// The user changed the address text since focusing it.
    pub typed: bool,
    logo: Option<egui::TextureHandle>,
}

impl UiState {
    pub fn new() -> Self {
        Self { address: String::new(), editing: false, focus_address: false, blur: false, shield_open: false, setup_open: false, setup_step: 0, tor: None, restart_needed: false, maximized: false, update: None, draft: crate::settings::Settings::recommended(), vault_open: false, vault_password: String::new(), popup_open: false, typed: false, logo: None }
    }
}

pub fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::path::Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let mut load = |key: &str, files: &[&str]| {
        for f in files {
            if let Ok(bytes) = std::fs::read(dir.join(f)) {
                fonts.font_data.insert(key.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                return true;
            }
        }
        false
    };
    let ui_font = load("ui", &["segoeui.ttf"]);
    let icons = load("icons", &["SegoeIcons.ttf", "segmdl2.ttf"]);
    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    if ui_font {
        prop.insert(0, "ui".into());
    }
    if icons {
        prop.push("icons".into());
        fonts.families.insert(FontFamily::Name("icons".into()), vec!["icons".into()]);
    } else {
        let p = fonts.families[&FontFamily::Proportional].clone();
        fonts.families.insert(FontFamily::Name("icons".into()), p);
    }
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = CHROME;
    v.window_fill = CHROME;
    v.extreme_bg_color = BG;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = RED.linear_multiply(0.5);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.hyperlink_color = RED_HI;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.weak_bg_fill = RAISED;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.inactive.corner_radius = CornerRadius::same(8);
    v.widgets.hovered.corner_radius = CornerRadius::same(8);
    v.widgets.active.corner_radius = CornerRadius::same(8);
    ctx.set_visuals(v);
}

fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

/// Turns what the user typed into a URL: an address, or a private DuckDuckGo search.
pub fn resolve_input(input: &str, settings: &crate::settings::Settings) -> Option<url::Url> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(u) = url::Url::parse(s) {
        if matches!(u.scheme(), "http" | "https" | "file" | "about" | "data") {
            return Some(u);
        }
    }
    let looks_like_host = !s.contains(' ')
        && (s.starts_with("localhost") || (s.contains('.') && !s.ends_with('.') && s.split('.').last().is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == ':' || c == '?' || c == '=' || c == '-' || c == '_' || c == '#' || c == '&' || c == '%'))));
    if looks_like_host {
        let scheme = if s.starts_with("localhost") || s.starts_with("127.") { "http" } else { "https" };
        if let Ok(u) = url::Url::parse(&format!("{scheme}://{s}")) {
            return Some(u);
        }
    }
    settings.search_url(s)
}

pub fn to_color_image(image: &servo::Image) -> egui::ColorImage {
    let (w, h) = (image.width as usize, image.height as usize);
    let d = image.data();
    match image.format {
        servo::PixelFormat::RGBA8 => egui::ColorImage::from_rgba_unmultiplied([w, h], d),
        servo::PixelFormat::BGRA8 => {
            let rgba: Vec<u8> = d.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
            egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)
        }
        servo::PixelFormat::RGB8 => egui::ColorImage::from_rgb([w, h], d),
        servo::PixelFormat::K8 => egui::ColorImage::from_gray([w, h], d),
        servo::PixelFormat::KA8 => {
            let rgba: Vec<u8> = d.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect();
            egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)
        }
    }
}

/// Small Bitwarden-style mark (blue shield) for the password-manager button.
pub fn bitwarden_icon(p: &egui::Painter, c: Pos2, size: f32, dim: bool) {
    let s = size / 2.0;
    let blue = if dim { Color32::from_rgb(70, 100, 150) } else { Color32::from_rgb(23, 93, 220) };
    let shield = |k: f32| -> Vec<Pos2> {
        let (w, top, mid, bot) = (s * 0.9 * k, -s * 0.95 * k, s * 0.25 * k, s * 1.0 * k);
        vec![
            c + vec2(-w, top), c + vec2(w, top), c + vec2(w, mid), c + vec2(w * 0.55, bot * 0.72), c + vec2(0.0, bot),
            c + vec2(-w * 0.55, bot * 0.72), c + vec2(-w, mid),
        ]
    };
    p.add(egui::Shape::convex_polygon(shield(1.0), blue, Stroke::NONE));
    // Inner half-shield, like Bitwarden's mark.
    let inner = shield(0.62);
    let half: Vec<Pos2> = inner.iter().map(|q| Pos2::new(q.x.max(c.x), q.y)).collect();
    p.add(egui::Shape::convex_polygon(half, Color32::WHITE.linear_multiply(if dim { 0.6 } else { 0.95 }), Stroke::NONE));
}

/// Small Tor-style onion mark.
pub fn tor_icon(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    let r = size / 2.0;
    p.circle_filled(c + vec2(0.0, r * 0.12), r * 0.88, color);
    p.circle_stroke(c + vec2(0.0, r * 0.12), r * 0.58, Stroke::new(1.3_f32, Color32::from_rgb(30, 18, 45)));
    p.circle_stroke(c + vec2(0.0, r * 0.12), r * 0.28, Stroke::new(1.3_f32, Color32::from_rgb(30, 18, 45)));
    // little sprout on top
    p.line_segment([c + vec2(0.0, -r * 0.7), c + vec2(r * 0.35, -r * 1.05)], Stroke::new(1.6_f32, color));
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

pub fn logo_texture(ui: &egui::Ui, state: &mut UiState) -> egui::TextureId {
    logo(ui, state)
}

fn logo(ui: &egui::Ui, state: &mut UiState) -> egui::TextureId {
    state
        .logo
        .get_or_insert_with(|| {
            let n = 128;
            ui.ctx().load_texture("rustwolf-logo", egui::ColorImage::from_rgba_unmultiplied([n, n], &crate::logo::render_rgba(n)), Default::default())
        })
        .id()
}

fn icon_button(ui: &mut egui::Ui, glyph: &str, enabled: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), if enabled { Sense::click() } else { Sense::hover() });
    let t = ui.ctx().animate_bool(resp.id, enabled && resp.hovered());
    if t > 0.0 {
        ui.painter().rect_filled(rect, 9.0, HOVER.linear_multiply(t));
    }
    let color = if enabled { TEXT } else { SUBTLE.linear_multiply(0.45) };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, icons(15.0), color);
    resp.on_hover_text(tip)
}

/// Draws the whole interface and returns the rectangle left for the web page (in points).
pub fn draw(root: &mut egui::Ui, st: &mut UiState, tabs: &[Tab], active: usize, blocker: &Blocker, shields_off: &HashSet<String>, vault: &crate::vault::Vault, settings: &crate::settings::Settings, actions: &mut Vec<Action>) -> Rect {
    if st.setup_open {
        crate::setup_ui::wizard(root, st, actions);
        return Rect::NOTHING;
    }
    let tab = &tabs[active];
    if !st.editing {
        st.address = tab.url.clone();
    }
    st.popup_open = false;

    // ---------- tab strip
    egui::Panel::top("tabs").exact_size(42.0).frame(Frame::new().fill(BG).inner_margin(Margin { left: 10, right: 10, top: 6, bottom: 0 })).show_inside(root, |ui| {
        ui.horizontal(|ui| {
            let lg = logo(ui, st);
            ui.add(egui::Image::new((lg, vec2(22.0, 22.0))));
            ui.add_space(6.0);
            let avail = ui.available_width() - 44.0 - 190.0; // room for "+", a drag area and the window buttons
            let w = (avail / tabs.len() as f32).clamp(60.0, 230.0);
            for (i, t) in tabs.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
                let is_active = i == active;
                let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
                let fill = if is_active { RAISED } else { HOVER.linear_multiply(hover * 0.7) };
                let r = CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 };
                ui.painter().rect_filled(rect, r, fill);
                if is_active {
                    ui.painter().line_segment([rect.left_top() + vec2(8.0, 0.5), rect.right_top() + vec2(-8.0, 0.5)], Stroke::new(2.0_f32, RED));
                }
                let mut x = rect.left() + 10.0;
                let cy = rect.center().y;
                if t.loading {
                    let ang = ui.input(|i| i.time) as f32 * 6.0;
                    let c = Pos2::new(x + 8.0, cy);
                    let pts: Vec<Pos2> = (0..12).map(|k| { let a = ang + k as f32 * 0.4; c + vec2(a.cos(), a.sin()) * 6.0 }).collect();
                    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.0_f32, RED)));
                } else if let Some(f) = &t.favicon {
                    ui.painter().image(f.id(), Rect::from_center_size(Pos2::new(x + 8.0, cy), vec2(16.0, 16.0)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                } else if t.webview.is_none() {
                    ui.painter().image(lg, Rect::from_center_size(Pos2::new(x + 8.0, cy), vec2(16.0, 16.0)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                } else {
                    ui.painter().text(Pos2::new(x + 8.0, cy), Align2::CENTER_CENTER, icon::GLOBE, icons(13.0), SUBTLE);
                }
                x += 24.0;
                let close = Rect::from_center_size(Pos2::new(rect.right() - 16.0, cy), vec2(22.0, 22.0));
                let title_w = (close.left() - x - 4.0).max(0.0);
                let galley = ui.painter().layout(t.title.clone(), FontId::proportional(13.0), if is_active { TEXT } else { SUBTLE }, f32::INFINITY);
                let clip = Rect::from_min_size(Pos2::new(x, rect.top()), vec2(title_w, rect.height()));
                ui.painter().with_clip_rect(clip).galley(Pos2::new(x, cy - galley.size().y / 2.0), galley, TEXT);
                let close_hover = ui.rect_contains_pointer(close);
                if close_hover {
                    ui.painter().rect_filled(close, 6.0, HOVER);
                }
                ui.painter().text(close.center(), Align2::CENTER_CENTER, icon::CLOSE, icons(9.0), SUBTLE);
                if resp.clicked() {
                    if close_hover {
                        actions.push(Action::CloseTab(i));
                    } else {
                        actions.push(Action::Select(i));
                    }
                }
                if resp.middle_clicked() {
                    actions.push(Action::CloseTab(i));
                }
            }
            if icon_button(ui, icon::ADD, true, "New tab (Ctrl+T)").clicked() {
                actions.push(Action::NewTab);
            }
            // Empty space = title bar: drag to move the window, double-click to maximize.
            let drag_w = (ui.available_width() - 3.0 * 46.0).max(0.0);
            let (_, drag) = ui.allocate_exact_size(vec2(drag_w, 36.0), Sense::click_and_drag());
            if drag.double_clicked() {
                actions.push(Action::ToggleMaximize);
            } else if drag.drag_started() {
                actions.push(Action::DragWindow);
            }
            // Window buttons, top right (like Firefox).
            let restore = st.maximized;
            for (glyph, action, close) in [
                ("\u{E921}", Action::Minimize, false),
                (if restore { "\u{E923}" } else { "\u{E922}" }, Action::ToggleMaximize, false),
                ("\u{E8BB}", Action::CloseWindow, true),
            ] {
                let (r, resp) = ui.allocate_exact_size(vec2(46.0, 36.0), Sense::click());
                let r = Rect::from_min_max(Pos2::new(r.left(), r.top() - 6.0), r.max); // reach the window's top edge
                if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, if close { Color32::from_rgb(196, 43, 28) } else { HOVER });
                }
                let col = if close && resp.hovered() { Color32::WHITE } else { TEXT };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, icons(10.0), col);
                if resp.clicked() {
                    actions.push(action);
                }
            }
        });
    });

    // ---------- toolbar
    egui::Panel::top("toolbar").exact_size(48.0).frame(Frame::new().fill(RAISED).inner_margin(Margin::symmetric(8, 7))).show_inside(root, |ui| {
        ui.horizontal(|ui| {
            let wv = tab.webview.as_ref();
            if icon_button(ui, icon::BACK, wv.is_some_and(|w| w.can_go_back()), "Back (Alt+Left)").clicked() {
                actions.push(Action::Back);
            }
            if icon_button(ui, icon::FORWARD, wv.is_some_and(|w| w.can_go_forward()), "Forward (Alt+Right)").clicked() {
                actions.push(Action::Forward);
            }
            if tab.loading {
                if icon_button(ui, icon::STOP, true, "Stop").clicked() {
                    actions.push(Action::Stop);
                }
            } else if icon_button(ui, icon::RELOAD, wv.is_some(), "Reload (Ctrl+R)").clicked() {
                actions.push(Action::Reload);
            }
            if icon_button(ui, icon::HOME, true, "Home").clicked() {
                actions.push(Action::Home);
            }
            ui.add_space(6.0);

            // Address bar.
            let host = host_of(&tab.url);
            let shield_on = !shields_off.contains(&host);
            if let Some((tag, url)) = &st.update {
                if ui.add(egui::Button::new(egui::RichText::new(format!("Update to {tag}")).size(12.5).color(Color32::WHITE)).fill(RED).corner_radius(10.0).min_size(vec2(110.0, 30.0))).on_hover_text("A newer RustWolf with security fixes is available").clicked() {
                    actions.push(Action::OpenInNewTab(url.clone()));
                }
            }
            // Restart button (after turning Tor on/off) and Tor status chip.
            if st.restart_needed {
                if ui.add(egui::Button::new(egui::RichText::new("Restart to apply").size(12.5).color(Color32::WHITE)).fill(RED).corner_radius(10.0).min_size(vec2(120.0, 30.0))).clicked() {
                    actions.push(Action::Restart);
                }
            }
            if let Some((ready, failed, status)) = &st.tor {
                let (txt, col) = if *ready {
                    ("Tor on".to_string(), Color32::from_rgb(170, 120, 255))
                } else if *failed {
                    ("Tor failed".to_string(), RED_HI)
                } else {
                    (status.trim_start_matches("Connecting to Tor… ").replace("Connecting to the Tor network…", "Tor…").replace("Downloading Tor from torproject.org…", "Tor ↓"), SUBTLE)
                };
                let label = txt.trim_start_matches("Tor").trim().to_string();
                let label = if label.is_empty() { "…".to_string() } else { label };
                let (ir, iresp) = ui.allocate_exact_size(vec2(18.0, 30.0), Sense::hover());
                tor_icon(ui.painter(), ir.center(), 15.0, Color32::from_rgb(125, 70, 152));
                iresp.on_hover_text(status.clone());
                ui.add(egui::Label::new(egui::RichText::new(label).size(12.5).strong().color(col)).selectable(false)).on_hover_text(status.clone());
            }
            let bar_w = ui.available_width() - 158.0;
            let (bar, _) = ui.allocate_exact_size(vec2(bar_w, 34.0), Sense::hover());
            let focused_id = egui::Id::new("address");
            let has_focus = ui.memory(|m| m.has_focus(focused_id));
            ui.painter().rect(bar, 12.0, BG, if has_focus { Stroke::new(1.5_f32, RED) } else { Stroke::new(1.0_f32, Color32::from_white_alpha(18)) }, egui::StrokeKind::Inside);
            let lock = if tab.url.starts_with("https://") { (icon::LOCK, SUBTLE) } else if tab.url.is_empty() { (icon::SEARCH, SUBTLE) } else { (icon::UNLOCK, RED_HI) };
            ui.painter().text(Pos2::new(bar.left() + 18.0, bar.center().y), Align2::CENTER_CENTER, lock.0, icons(13.0), lock.1);
            let edit_rect = Rect::from_min_max(Pos2::new(bar.left() + 34.0, bar.top() + 4.0), Pos2::new(bar.right() - 10.0, bar.bottom() - 4.0));
            let resp = ui
                .scope_builder(egui::UiBuilder::new().max_rect(edit_rect).layout(Layout::left_to_right(Align::Center)), |ui| {
                    ui.add(
                        TextEdit::singleline(&mut st.address)
                            .id(focused_id)
                            .frame(Frame::NONE)
                            .hint_text(format!("Search with {} or type an address", settings.engine().name))
                            .font(FontId::proportional(14.0))
                            .desired_width(edit_rect.width()),
                    )
                })
                .inner;
            if st.focus_address {
                resp.request_focus();
                if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), focused_id) {
                    s.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(st.address.chars().count()))));
                    s.store(ui.ctx(), focused_id);
                }
                st.focus_address = false;
                st.editing = true;
            }
            if st.blur {
                resp.surrender_focus();
                st.blur = false;
            }
            if resp.changed() {
                st.typed = true;
            }
            if !resp.has_focus() {
                st.typed = false;
            }
            if resp.changed() || resp.has_focus() {
                st.editing = resp.has_focus() || resp.changed();
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    actions.push(Action::Navigate(st.address.clone()));
                } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                    st.address = tab.url.clone();
                }
                st.editing = false;
            }

            // Bitwarden button: shows how many saved logins match this site.
            ui.add_space(6.0);
            let (krect, kresp) = ui.allocate_exact_size(vec2(46.0, 34.0), Sense::click());
            let kh = ui.ctx().animate_bool(kresp.id, kresp.hovered() || st.vault_open);
            ui.painter().rect_filled(krect, 9.0, HOVER.linear_multiply(kh));
            let unlocked = vault.state == crate::vault::State::Unlocked;
            let kcol = if unlocked && !vault.matches.is_empty() { RED } else if unlocked { TEXT } else { SUBTLE };
            let _ = kcol;
            bitwarden_icon(ui.painter(), krect.center() - vec2(6.0, 0.0), 18.0, !unlocked);
            if unlocked && !vault.matches.is_empty() {
                let badge = Rect::from_center_size(krect.center() + vec2(11.0, 7.0), vec2(18.0, 15.0));
                ui.painter().rect_filled(badge, 7.0, RED);
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, vault.matches.len().to_string(), FontId::proportional(10.0), Color32::WHITE);
            }
            if kresp.on_hover_text("Bitwarden logins").clicked() {
                st.vault_open = !st.vault_open;
                st.shield_open = false;
                if st.vault_open {
                    actions.push(Action::VaultCheck);
                }
            }

            // Shield button with the blocked counter.
            let (srect, sresp) = ui.allocate_exact_size(vec2(46.0, 34.0), Sense::click());
            let h = ui.ctx().animate_bool(sresp.id, sresp.hovered() || st.shield_open);
            ui.painter().rect_filled(srect, 9.0, HOVER.linear_multiply(h));
            let scol = if shield_on { RED } else { SUBTLE };
            ui.painter().text(srect.center() - vec2(6.0, 0.0), Align2::CENTER_CENTER, icon::SHIELD, icons(17.0), scol);
            if shield_on && tab.blocked.total > 0 {
                let badge = Rect::from_center_size(srect.center() + vec2(11.0, 7.0), vec2(22.0, 15.0));
                ui.painter().rect_filled(badge, 7.0, RED);
                let n = if tab.blocked.total > 99 { "99+".to_string() } else { tab.blocked.total.to_string() };
                ui.painter().text(badge.center(), Align2::CENTER_CENTER, n, FontId::proportional(10.0), Color32::WHITE);
            }
            if sresp.on_hover_text("Tracker shield").clicked() {
                st.shield_open = !st.shield_open;
                st.vault_open = false;
            }
            if icon_button(ui, "\u{E713}", true, "Settings").clicked() {
                actions.push(Action::OpenSettings);
            }
        });
    });

    // ---------- loading bar
    let (bar_rect, _) = root.allocate_exact_size(vec2(root.available_width(), 2.0), Sense::hover());
    if tab.loading {
        let t = (root.input(|i| i.time) as f32 * 0.9).fract();
        let w = bar_rect.width() * 0.3;
        let x = bar_rect.left() - w + (bar_rect.width() + w) * t;
        root.painter().with_clip_rect(bar_rect).rect_filled(Rect::from_min_size(Pos2::new(x, bar_rect.top()), vec2(w, 2.0)), 1.0, RED);
    }

    // ---------- Bitwarden panel
    if st.vault_open {
        egui::Panel::right("vault").exact_size(300.0).frame(Frame::new().fill(CHROME).inner_margin(Margin::same(16))).show_inside(root, |ui| {
            vault_panel(ui, st, vault, &tab.url, actions);
        });
    }

    // ---------- shield panel (takes space from the page, never covers it)
    if st.shield_open {
        egui::Panel::right("shield").exact_size(300.0).frame(Frame::new().fill(CHROME).inner_margin(Margin::same(16))).show_inside(root, |ui| {
            let host = host_of(&tab.url);
            let on = !shields_off.contains(&host);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icon::SHIELD).font(icons(20.0)).color(if on { RED } else { SUBTLE }));
                ui.label(egui::RichText::new("Tracker shield").size(17.0).strong());
            });
            ui.add_space(8.0);
            if host.is_empty() {
                ui.label(egui::RichText::new("Open a website to see what gets blocked.").color(SUBTLE));
            } else {
                ui.label(egui::RichText::new(&host).color(SUBTLE));
                ui.add_space(6.0);
                let label = if on { "Protection is ON for this site" } else { "Protection is OFF for this site" };
                if ui.add(egui::Button::new(label).fill(if on { RED.linear_multiply(0.35) } else { HOVER }).min_size(vec2(268.0, 32.0))).clicked() {
                    actions.push(Action::ToggleShield);
                }
                ui.add_space(12.0);
                ui.label(egui::RichText::new(format!("{}", tab.blocked.total)).size(34.0).color(RED).strong());
                ui.label(egui::RichText::new("trackers and ads blocked on this page").color(SUBTLE));
                ui.add_space(8.0);
                let site = crate::base_domain(&host);
                let scripts_blocked = settings.no_3p_scripts.contains(&site);
                let label = if scripts_blocked { "Third-party scripts: BLOCKED here" } else { "Third-party scripts: allowed here" };
                if ui.add(egui::Button::new(label).fill(if scripts_blocked { RED.linear_multiply(0.35) } else { HOVER }).min_size(vec2(268.0, 30.0))).on_hover_text("Blocks script files from other sites (most tracking code). Scripts written into the page itself still run.").clicked() {
                    actions.push(Action::ToggleScripts);
                }
                ui.add_space(10.0);
                egui::ScrollArea::vertical().max_height(ui.available_height() - 90.0).show(ui, |ui| {
                    for (h, n) in &tab.blocked.by_host {
                        ui.horizontal(|ui| {
                            ui.label(h);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(egui::RichText::new(n.to_string()).color(SUBTLE)));
                        });
                    }
                });
            }
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.label(egui::RichText::new("HTTPS-only · tracking links cleaned · fingerprint protection · WebRTC, location, camera & mic off · nothing saved to disk").size(11.0).color(SUBTLE));
                ui.label(egui::RichText::new(format!("{} rules: {}", blocker.rule_count, blocker.lists.join(", "))).size(11.0).color(SUBTLE));
            });
        });
    }

    // ---------- ClickFix warning bar
    if let Some(cmd) = &tab.alert {
        egui::Panel::top("clickfix").frame(Frame::new().fill(Color32::from_rgb(120, 20, 26)).inner_margin(Margin::symmetric(14, 10))).show_inside(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icon::SHIELD).font(icons(20.0)).color(Color32::WHITE));
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 250.0);
                    ui.label(egui::RichText::new("RustWolf blocked this site from copying a command to your clipboard").strong().color(Color32::WHITE));
                    ui.label(egui::RichText::new("This is a known scam (\"ClickFix\"): fake CAPTCHA or error pages tell you to press Win+R and paste. Never paste website instructions into Win+R, PowerShell or a terminal.").size(12.5).color(Color32::from_rgb(255, 210, 210)));
                    let preview: String = cmd.chars().take(140).collect();
                    ui.label(egui::RichText::new(preview).monospace().size(11.5).color(Color32::from_rgb(255, 230, 230)));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(egui::RichText::new("OK, keep me safe").color(Color32::WHITE)).fill(RED).min_size(vec2(130.0, 30.0))).clicked() {
                        actions.push(Action::AlertDismiss);
                    }
                    if ui.add(egui::Button::new(egui::RichText::new("Copy anyway").size(12.0)).frame(false)).on_hover_text("Only if you trust this site and know what the command does").clicked() {
                        actions.push(Action::AlertCopy);
                    }
                });
            });
        });
    }

    // ---------- page area / new-tab page
    let mut page = Rect::NOTHING;
    egui::CentralPanel::default().frame(Frame::new().fill(BG)).show_inside(root, |ui| {
        page = ui.max_rect();
        // Letterboxing: the page gets a size rounded to 100 px steps, so your exact window size
        // can't be used to recognise you (like Tor Browser / Mullvad Browser).
        if settings.fingerprint && tab.webview.is_some() {
            let w = ((page.width() / 100.0).floor() * 100.0).max(200.0).min(page.width());
            let h = ((page.height() / 100.0).floor() * 100.0).max(200.0).min(page.height());
            page = Rect::from_center_size(page.center(), vec2(w, h));
        }
        if let (None, Some(n)) = (&tab.webview, &tab.notice) {
            notice_page(ui, n, actions);
        } else if tab.webview.is_none() {
            new_tab_page(ui, st, actions);
        } else if let Some(s) = &tab.status {
            let galley = ui.painter().layout_no_wrap(s.clone(), FontId::proportional(12.0), TEXT);
            let r = Rect::from_min_size(Pos2::new(page.left() + 6.0, page.bottom() - 26.0), galley.size() + vec2(14.0, 8.0));
            ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("status"))).rect_filled(r, 6.0, RAISED);
            ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("status"))).galley(r.min + vec2(7.0, 4.0), galley, TEXT);
        }
    });
    page
}

fn vault_panel(ui: &mut egui::Ui, st: &mut UiState, vault: &crate::vault::Vault, url: &str, actions: &mut Vec<Action>) {
    use crate::vault::State;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        bitwarden_icon(ui.painter(), r.center(), 20.0, false);
        ui.label(egui::RichText::new("Bitwarden").size(17.0).strong());
    });
    ui.add_space(8.0);
    let small = |t: &str| egui::RichText::new(t.to_string()).size(12.5).color(SUBTLE);
    match vault.state {
        State::Checking => {
            ui.label(small("Checking Bitwarden…"));
        }
        State::NoCli => {
            ui.label("RustWolf fills your Bitwarden logins through Bitwarden's official command-line tool.");
            ui.add_space(6.0);
            ui.label(small("Install it once (in PowerShell):"));
            ui.add(egui::TextEdit::singleline(&mut "winget install Bitwarden.CLI".to_string()).font(FontId::monospace(12.0)).desired_width(268.0));
            ui.label(small("Then log in once with:  bw login"));
            ui.add_space(8.0);
            if ui.button("Check again").clicked() {
                actions.push(Action::VaultCheck);
            }
        }
        State::LoggedOut => {
            ui.label("Bitwarden CLI is installed but not logged in.");
            ui.label(small("Log in once in PowerShell with:  bw login"));
            ui.add_space(8.0);
            if ui.button("Check again").clicked() {
                actions.push(Action::VaultCheck);
            }
        }
        State::Locked => {
            if let Some(e) = &vault.email {
                ui.label(small(e));
            }
            ui.label("Unlock your vault to fill logins on this site.");
            ui.add_space(6.0);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut st.vault_password)
                    .password(true)
                    .hint_text("Master password")
                    .desired_width(268.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(6.0);
            let label = if vault.busy { "Unlocking…" } else { "Unlock" };
            if (ui.add_enabled(!vault.busy, egui::Button::new(egui::RichText::new(label).color(Color32::WHITE)).fill(RED).min_size(vec2(268.0, 32.0))).clicked() || enter)
                && !st.vault_password.is_empty()
            {
                actions.push(Action::VaultUnlock);
            }
            ui.label(small("Your master password goes straight to Bitwarden's tool and is never stored."));
        }
        State::Unlocked => {
            let host = host_of(url);
            if host.is_empty() {
                ui.label(small("Open a website to see its saved logins."));
            } else if vault.matches.is_empty() {
                ui.label(small(&format!("No saved logins for {host}.")));
            } else {
                ui.label(small(&format!("Saved logins for {host}:")));
                ui.add_space(4.0);
                for (i, l) in vault.matches.iter().enumerate() {
                    egui::Frame::new().fill(RAISED).corner_radius(10.0).inner_margin(Margin::same(10)).show(ui, |ui| {
                        ui.set_width(256.0);
                        ui.label(egui::RichText::new(if l.name.is_empty() { &l.site } else { &l.name }).strong());
                        ui.label(small(if l.username.is_empty() { "(no username)" } else { &l.username }));
                        if ui.add(egui::Button::new(egui::RichText::new("Fill").color(Color32::WHITE)).fill(RED).min_size(vec2(80.0, 28.0))).clicked() {
                            actions.push(Action::VaultFill(i));
                        }
                    });
                    ui.add_space(4.0);
                }
            }
            ui.add_space(10.0);
            if ui.button("Lock vault").clicked() {
                actions.push(Action::VaultLock);
            }
        }
    }
    if let Some(e) = &vault.error {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(e).size(12.5).color(RED_HI));
    }
}

fn notice_page(ui: &mut egui::Ui, n: &crate::Notice, actions: &mut Vec<Action>) {
    let rect = ui.max_rect();
    let w = 560.0_f32.min(rect.width() - 40.0);
    let area = Rect::from_center_size(rect.center(), vec2(w, 260.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Center)), |ui| {
        ui.label(egui::RichText::new(icon::SHIELD).font(icons(46.0)).color(if n.danger { RED } else { SUBTLE }));
        ui.add_space(10.0);
        ui.label(egui::RichText::new(&n.title).size(22.0).strong());
        ui.add_space(8.0);
        ui.label(egui::RichText::new(&n.text).size(14.0).color(SUBTLE));
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space((w - 340.0).max(0.0) / 2.0);
            if n.danger {
                if ui.add(egui::Button::new(egui::RichText::new("Go back to safety").color(Color32::WHITE)).fill(RED).min_size(vec2(170.0, 36.0))).clicked() {
                    actions.push(Action::Home);
                }
                if ui.add(egui::Button::new("Open anyway (unsafe)").min_size(vec2(160.0, 36.0))).clicked() {
                    actions.push(Action::Proceed(n.url.clone()));
                }
            } else {
                if ui.add(egui::Button::new(egui::RichText::new("Reload page").color(Color32::WHITE)).fill(RED).min_size(vec2(170.0, 36.0))).clicked() {
                    actions.push(Action::Navigate(n.url.clone()));
                }
                if ui.add(egui::Button::new("New tab page").min_size(vec2(160.0, 36.0))).clicked() {
                    actions.push(Action::Home);
                }
            }
        });
    });
}

fn new_tab_page(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    let rect = ui.max_rect();
    let p = ui.painter();
    // soft red glows
    for (c, r, a) in [(rect.left_top() + vec2(rect.width() * 0.15, rect.height() * 0.1), 520.0, 0.10), (rect.right_bottom() - vec2(rect.width() * 0.1, rect.height() * 0.1), 460.0, 0.12)] {
        for k in 0..12 {
            let f = 1.0 - k as f32 / 12.0;
            p.circle_filled(c, r * f, RED.linear_multiply(a * (1.0 - f) * 0.35));
        }
    }
    let lg = logo(ui, st);
    let center = rect.center() - vec2(0.0, 90.0);
    p.image(lg, Rect::from_center_size(center, vec2(120.0, 120.0)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
    let title_y = center.y + 88.0;
    let rust = p.layout_no_wrap("Rust".into(), FontId::proportional(40.0), TEXT);
    let wolf = p.layout_no_wrap("Wolf".into(), FontId::proportional(40.0), RED);
    let total = rust.size().x + wolf.size().x;
    let x0 = center.x - total / 2.0;
    p.galley(Pos2::new(x0, title_y - rust.size().y / 2.0), rust.clone(), TEXT);
    p.galley(Pos2::new(x0 + rust.size().x, title_y - wolf.size().y / 2.0), wolf, RED);
    p.text(Pos2::new(center.x, title_y + 36.0), Align2::CENTER_CENTER, "Private by default. Written in Rust.", FontId::proportional(14.0), SUBTLE);

    let chips = ["Ad & tracker blocking", "HTTPS-only", "Fingerprint protection", "Clean links", "Malware & phishing shield", "Nothing saved to disk", "Servo engine (Rust)"];
    let y = title_y + 80.0;
    let widths: Vec<f32> = chips.iter().map(|c| p.layout_no_wrap((*c).into(), FontId::proportional(12.5), TEXT).size().x + 26.0).collect();
    let mut x = center.x - (widths.iter().sum::<f32>() + 8.0 * (chips.len() - 1) as f32) / 2.0;
    for (c, w) in chips.iter().zip(widths) {
        let r = Rect::from_min_size(Pos2::new(x, y), vec2(w, 28.0));
        p.rect(r, 14.0, Color32::from_white_alpha(10), Stroke::new(1.0_f32, Color32::from_white_alpha(20)), egui::StrokeKind::Inside);
        p.text(r.center(), Align2::CENTER_CENTER, *c, FontId::proportional(12.5), TEXT);
        x += w + 8.0;
    }
    let _ = actions;
}
