//! First-run setup (and Settings): search engine, protection level, privacy tools.

use egui::{vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, Sense, Stroke};

use crate::settings::{Settings, ENGINES};
use crate::ui::{UiState, BG, HOVER, RAISED, RED, RED_HI, SUBTLE, TEXT};
use crate::Action;

const STEPS: usize = 5;

/// An iOS-style switch.
fn switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = crate::ui::lerp_color(Color32::from_rgb(60, 60, 72), RED, t);
    ui.painter().rect_filled(rect, 12.0, bg);
    let x = rect.left() + 12.0 + t * (rect.width() - 24.0);
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), 9.0, Color32::WHITE);
    resp
}

fn heading(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(egui::RichText::new(title).size(30.0).strong().color(TEXT));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(sub).size(14.5).color(SUBTLE));
    ui.add_space(22.0);
}

fn primary(ui: &mut egui::Ui, text: &str, width: f32) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(text).size(15.0).strong().color(Color32::WHITE)).fill(RED).corner_radius(12.0).min_size(vec2(width, 42.0))).clicked()
}

fn ghost(ui: &mut egui::Ui, text: &str, width: f32) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(text).size(14.0).color(TEXT)).fill(RAISED).corner_radius(12.0).min_size(vec2(width, 42.0))).clicked()
}

fn toggle_row(ui: &mut egui::Ui, on: &mut bool, title: &str, desc: &str) {
    Frame::new().fill(RAISED).corner_radius(14.0).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 60.0);
                ui.label(egui::RichText::new(title).size(15.0).strong().color(TEXT));
                ui.label(egui::RichText::new(desc).size(12.5).color(SUBTLE));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                switch(ui, on);
            });
        });
    });
    ui.add_space(8.0);
}

fn glows(ui: &egui::Ui, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, BG);
    for (c, r, a) in [(rect.left_top() + vec2(rect.width() * 0.12, rect.height() * 0.15), 560.0, 0.12), (rect.right_bottom() - vec2(rect.width() * 0.1, rect.height() * 0.12), 520.0, 0.14)] {
        for k in 0..14 {
            let f = 1.0 - k as f32 / 14.0;
            p.circle_filled(c, r * f, RED.linear_multiply(a * (1.0 - f) * 0.3));
        }
    }
}

/// Draws the setup screen over the whole window.
pub fn wizard(root: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    let full = root.max_rect();
    glows(root, full);
    let first_run = !st.draft.setup_done;
    let step = st.setup_step.min(STEPS - 1);

    // Slide + fade between steps.
    let t = root.ctx().animate_value_with_time(egui::Id::new("setup-step"), step as f32, 0.25);
    let offset = (step as f32 - t) * 60.0;
    let fade = 1.0 - (step as f32 - t).abs().min(1.0) * 0.8;

    let w = 760.0_f32.min(full.width() - 48.0);
    let content = Rect::from_min_size(Pos2::new(full.center().x - w / 2.0 + offset, full.top() + 56.0), vec2(w, full.height() - 150.0));
    let mut done_clicked = false;
    root.scope_builder(egui::UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Min)), |ui| {
        ui.multiply_opacity(fade);
        egui::ScrollArea::vertical().max_height(content.height()).show(ui, |ui| {
            ui.set_width(w);
            match step {
                0 => welcome(ui, st, actions, &mut done_clicked),
                1 => search(ui, st),
                2 => protection(ui, st),
                3 => tools(ui, st),
                _ => summary(ui, st),
            }
        });
    });

    // Bottom bar: back · step dots · next.
    let bar = Rect::from_min_size(Pos2::new(full.center().x - w / 2.0, full.bottom() - 82.0), vec2(w, 50.0));
    root.scope_builder(egui::UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(Align::Center)), |ui| {
        if step > 0 {
            if ghost(ui, "Back", 120.0) {
                st.setup_step -= 1;
            }
        } else if !first_run {
            if ghost(ui, "Cancel", 120.0) {
                st.setup_open = false;
            }
        } else {
            ui.add_space(120.0 + ui.spacing().item_spacing.x);
        }
        // Dots
        let dots_w = w - 2.0 * 120.0 - 40.0;
        let (dots, _) = ui.allocate_exact_size(vec2(dots_w, 42.0), Sense::hover());
        for i in 0..STEPS {
            let c = Pos2::new(dots.center().x + (i as f32 - (STEPS as f32 - 1.0) / 2.0) * 18.0, dots.center().y);
            let active = i == step;
            ui.painter().rect_filled(Rect::from_center_size(c, vec2(if active { 22.0 } else { 8.0 }, 8.0)), 4.0, if active { RED } else { HOVER });
        }
        if step + 1 < STEPS {
            if step > 0 && primary(ui, "Next", 120.0) {
                st.setup_step += 1;
            }
        } else if primary(ui, if first_run { "Start browsing" } else { "Save" }, 140.0) {
            done_clicked = true;
        }
    });

    if done_clicked {
        let mut s = st.draft.clone();
        s.setup_done = true;
        actions.push(Action::ApplySettings(Box::new(s)));
        st.setup_open = false;
    }
}

fn welcome(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>, done: &mut bool) {
    let _ = actions;
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        let lg = crate::ui::logo_texture(ui, st);
        ui.add(egui::Image::new((lg, vec2(150.0, 150.0))));
        ui.add_space(16.0);
        ui.label(egui::RichText::new("Welcome to RustWolf").size(36.0).strong().color(TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("A private browser written in Rust. Let's set it up, it takes less than a minute.").size(15.0).color(SUBTLE));
        ui.add_space(30.0);
        if primary(ui, "Get started", 240.0) {
            st.setup_step = 1;
        }
        ui.add_space(10.0);
        if ui.add(egui::Button::new(egui::RichText::new("Skip and use the recommended settings").size(13.0).color(SUBTLE)).frame(false)).clicked() {
            st.draft = Settings::recommended();
            *done = true;
        }
    });
}

fn search(ui: &mut egui::Ui, st: &mut UiState) {
    heading(ui, "Choose your search engine", "What you type in the address bar is searched here. You can change it anytime in Settings.");
    let cols = 3;
    let gap = 12.0;
    let card_w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    for row in ENGINES.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for e in row {
                let selected = st.draft.search == e.id;
                let (rect, resp) = ui.allocate_exact_size(vec2(card_w, 118.0), Sense::click());
                let h = ui.ctx().animate_bool(resp.id, resp.hovered() || selected);
                let fill = crate::ui::lerp_color(RAISED, HOVER, h);
                ui.painter().rect(rect, 16.0, fill, if selected { Stroke::new(2.0_f32, RED) } else { Stroke::new(1.0_f32, Color32::from_white_alpha(14)) }, egui::StrokeKind::Inside);
                let c = Color32::from_rgb(e.color[0], e.color[1], e.color[2]);
                let badge = Pos2::new(rect.left() + 32.0, rect.top() + 34.0);
                ui.painter().circle_filled(badge, 17.0, c);
                ui.painter().text(badge, Align2::CENTER_CENTER, &e.name[..1], FontId::proportional(17.0), Color32::WHITE);
                ui.painter().text(Pos2::new(rect.left() + 58.0, rect.top() + 34.0), Align2::LEFT_CENTER, e.name, FontId::proportional(16.0), TEXT);
                if selected {
                    ui.painter().text(Pos2::new(rect.right() - 18.0, rect.top() + 34.0), Align2::CENTER_CENTER, "\u{E73E}", FontId::new(15.0, egui::FontFamily::Name("icons".into())), RED_HI);
                }
                let galley = ui.painter().layout(e.blurb.to_string(), FontId::proportional(12.5), SUBTLE, card_w - 32.0);
                ui.painter().galley(Pos2::new(rect.left() + 16.0, rect.top() + 62.0), galley, SUBTLE);
                if resp.clicked() {
                    st.draft.search = e.id.to_string();
                }
            }
        });
        ui.add_space(gap);
    }
}

fn protection(ui: &mut egui::Ui, st: &mut UiState) {
    heading(ui, "Choose your protection", "Recommended blocks trackers without breaking sites. Strict blocks even more.");
    ui.horizontal(|ui| {
        let strict = st.draft.is_strict();
        for (label, is_strict) in [("Recommended", false), ("Strict", true)] {
            let selected = strict == is_strict;
            let btn = egui::Button::new(egui::RichText::new(label).size(14.0).strong().color(if selected { Color32::WHITE } else { TEXT }))
                .fill(if selected { RED } else { RAISED })
                .corner_radius(CornerRadius::same(12))
                .min_size(vec2(160.0, 38.0));
            if ui.add(btn).clicked() {
                if is_strict {
                    st.draft.strict_protection();
                } else {
                    st.draft.recommended_protection();
                }
            }
        }
    });
    ui.add_space(14.0);
    let d = &mut st.draft;
    toggle_row(ui, &mut d.block_ads, "Block ads and trackers", "EasyList + EasyPrivacy with Brave's Rust ad-block engine, plus hiding empty ad boxes.");
    toggle_row(ui, &mut d.cookie_banners, "Block cookie banners", "Hides most \"accept cookies\" pop-ups (Fanboy's cookie list).");
    toggle_row(ui, &mut d.annoyances, "Block annoyances", "Newsletter pop-ups, chat bubbles, \"open in app\" banners and similar clutter.");
    toggle_row(ui, &mut d.social, "Block social widgets", "Like / Share / Follow buttons that track you across sites. May hide some embedded posts.");
    toggle_row(ui, &mut d.malware, "Malware & phishing shield", "Warns you before opening known dangerous sites. Checked on your PC, nothing is sent anywhere.");
    toggle_row(ui, &mut d.fingerprint, "Fingerprint protection", "Makes your browser look like everyone else's so sites can't recognise you.");
    toggle_row(ui, &mut d.clean_links, "Clean tracking links", "Removes utm_, fbclid, gclid and other trackers from links you open.");
    toggle_row(ui, &mut d.https_only, "HTTPS-only", "Always use encrypted connections. (Applies fully after restarting RustWolf.)");
}

fn tools(ui: &mut egui::Ui, st: &mut UiState) {
    heading(
        ui,
        "Privacy tools",
        "Tor hides your IP address (on by default). Private front-ends open popular sites with the same content, no trackers and no account needed (off by default).",
    );
    let d = &mut st.draft;
    toggle_row(
        ui,
        &mut d.tor,
        "Connect through Tor",
        "On by default. Every site sees a Tor address instead of your real IP and location, each site gets its own Tor route, and .onion sites open. RustWolf downloads Tor (24 MB) from torproject.org and checks it. Pages load slower and some sites block Tor. It hides who you are on the network, but RustWolf is not as anonymous as Tor Browser.",
    );
    toggle_row(ui, &mut d.redirect_reddit, "Reddit → Redlib", "Read Reddit without its trackers and ads (via safereddit.com).");
    toggle_row(ui, &mut d.redirect_youtube, "YouTube → Invidious", "Browse YouTube without Google tracking (via yewtu.be). Video playback isn't supported in RustWolf yet.");
    ui.add_space(4.0);
    ui.label(egui::RichText::new("These are free community-run services. They can sometimes be slow or offline; turn them off here anytime.").size(12.0).color(SUBTLE));
}

fn summary(ui: &mut egui::Ui, st: &mut UiState) {
    let first_run = !st.draft.setup_done;
    heading(ui, if first_run { "You're all set" } else { "Review your settings" }, "Here's how RustWolf will protect you. Change anything later with the gear button.");
    let d = &st.draft;
    let mut items: Vec<String> = vec![format!("Search: {}", d.engine().name)];
    for (on, label) in [
        (d.block_ads, "Ads & trackers blocked"),
        (d.cookie_banners, "Cookie banners blocked"),
        (d.annoyances, "Annoyances blocked"),
        (d.social, "Social widgets blocked"),
        (d.malware, "Malware & phishing shield"),
        (d.fingerprint, "Fingerprint protection"),
        (d.clean_links, "Tracking links cleaned"),
        (d.https_only, "HTTPS-only"),
        (d.redirect_reddit, "Reddit → Redlib"),
        (d.redirect_youtube, "YouTube → Invidious"),
        (d.tor, "All traffic through Tor"),
    ] {
        if on {
            items.push(label.to_string());
        }
    }
    items.push("Nothing saved to disk".into());
    items.push("No telemetry".into());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for it in items {
            Frame::new().fill(RAISED).corner_radius(14.0).inner_margin(Margin::symmetric(12, 7)).stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(16))).show(ui, |ui| {
                ui.label(egui::RichText::new(format!("\u{2713}  {it}")).size(13.5).color(TEXT));
            });
        }
    });
}
