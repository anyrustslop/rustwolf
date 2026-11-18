//! Ad, tracker, cookie-banner, malware and phishing blocking, using Brave's Rust ad-block engine
//! (`adblock` crate, MPL-2.0) and well-known open filter lists.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Lists RustWolf uses besides its own built-in one. Downloaded on first start, refreshed weekly.
const BLOCK_LISTS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt"),
    ("cookiemonster.txt", "https://secure.fanboy.co.nz/fanboy-cookiemonster.txt"),
    ("annoyance.txt", "https://secure.fanboy.co.nz/fanboy-annoyance.txt"),
    ("social.txt", "https://easylist.to/easylist/fanboy-social.txt"),
];

/// Which lists the user's settings ask for.
fn wanted(name: &str, s: &crate::settings::Settings) -> bool {
    match name {
        "cookiemonster.txt" => s.cookie_banners,
        "annoyance.txt" => s.annoyances,
        "social.txt" => s.social,
        _ => true,
    }
}
/// Known malware and phishing sites: opening one shows a warning instead of the page.
const DANGER_LISTS: &[(&str, &str)] = &[
    ("urlhaus-domains.txt", "https://malware-filter.gitlab.io/malware-filter/urlhaus-filter-agh-online.txt"),
    ("phishing-domains.txt", "https://malware-filter.gitlab.io/malware-filter/phishing-filter-agh.txt"),
];
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

pub struct Blocker {
    pub engine: adblock::Engine,
    danger: Option<adblock::Engine>,
    pub rule_count: usize,
    pub lists: Vec<&'static str>,
}

fn count_rules(text: &str) -> usize {
    text.lines().filter(|l| { let l = l.trim(); !l.is_empty() && !l.starts_with('!') && !l.starts_with('[') }).count()
}

fn lists_dir() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("lists"))
}

impl Blocker {
    /// The built-in list only (used until the full lists are ready).
    pub fn builtin() -> Self {
        let text = Self::builtin_text();
        Self { rule_count: count_rules(&text), engine: adblock::Engine::new_with_list_text(text), danger: None, lists: vec!["RustWolf built-in"] }
    }

    fn builtin_text() -> String {
        let mut rules = include_str!("../assets/filters.txt").to_string();
        if let Some(extra) = crate::data_dir().map(|d| d.join("filters.txt")) {
            if let Ok(text) = std::fs::read_to_string(extra) {
                rules.push('\n');
                rules.push_str(&text);
            }
        }
        rules
    }

    /// Loads cached lists (downloading missing/old ones), then builds the full engines.
    /// Runs on a background thread.
    pub fn full(download: bool, settings: &crate::settings::Settings) -> Self {
        let dir = lists_dir();
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
        }
        let fetch = |name: &str, url: &str| -> Option<String> {
            let path = dir.as_ref()?.join(name);
            let fresh = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age < MAX_AGE);
            if !fresh && download {
                let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
                if let Ok(resp) = agent.get(url).call() {
                    let mut body = String::new();
                    if std::io::Read::read_to_string(&mut resp.into_reader(), &mut body).is_ok() {
                        if body.len() > 1000 {
                            let _ = std::fs::write(&path, &body);
                            return Some(body);
                        }
                    }
                }
            }
            std::fs::read_to_string(&path).ok()
        };

        let mut text = Self::builtin_text();
        let mut lists = vec!["RustWolf built-in"];
        for (name, url) in BLOCK_LISTS {
            if !wanted(name, settings) {
                continue;
            }
            if let Some(t) = fetch(name, url) {
                text.push('\n');
                text.push_str(&t);
                lists.push(match *name {
                    "easylist.txt" => "EasyList",
                    "easyprivacy.txt" => "EasyPrivacy",
                    "annoyance.txt" => "Annoyances (Fanboy)",
                    "social.txt" => "Social widgets (Fanboy)",
                    _ => "Cookie banners (Fanboy)",
                });
            }
        }
        let mut danger_text = String::new();
        for (name, url) in DANGER_LISTS {
            if let Some(t) = fetch(name, url) {
                danger_text.push('\n');
                danger_text.push_str(&t);
                lists.push(if *name == "urlhaus-domains.txt" { "Malware (URLhaus)" } else { "Phishing" });
            }
        }
        let rule_count = count_rules(&text) + count_rules(&danger_text);
        let danger = (!danger_text.is_empty()).then(|| adblock::Engine::new_with_list_text(danger_text));
        Self { engine: adblock::Engine::new_with_list_text(text), danger, rule_count, lists }
    }

    /// True when some list is missing or older than a week (needs a download).
    pub fn needs_update(settings: &crate::settings::Settings) -> bool {
        let Some(dir) = lists_dir() else { return false };
        BLOCK_LISTS.iter().filter(|(n, _)| wanted(n, settings)).chain(DANGER_LISTS).any(|(name, _)| {
            std::fs::metadata(dir.join(name))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_none_or(|age| age >= MAX_AGE)
        })
    }

    /// Should this sub-resource be blocked? `page` is the top-level page URL.
    pub fn should_block(&self, url: &str, page: &str, kind: &str) -> bool {
        match adblock::request::Request::new(url, page, kind, "GET") {
            Ok(req) => self.engine.check_network_request(&req).should_block(),
            Err(_) => false,
        }
    }

    /// Is this page a known malware or phishing site?
    pub fn is_dangerous(&self, url: &str) -> bool {
        let Some(d) = &self.danger else { return false };
        match adblock::request::Request::new(url, url, "document", "GET") {
            Ok(req) => d.check_network_request(&req).should_block(),
            Err(_) => false,
        }
    }

    /// CSS that hides ad boxes on this page (site-specific + generic rules).
    pub fn cosmetic_css(&self, url: &str) -> (String, bool, HashSet<String>) {
        let res = self.engine.url_cosmetic_resources(url);
        (selectors_to_css(res.hide_selectors.iter()), res.generichide, res.exceptions)
    }

    /// CSS for generic rules that match the classes/ids actually present on the page.
    pub fn class_id_css(&self, classes: &[String], ids: &[String], exceptions: &HashSet<String>) -> String {
        let sel = self.engine.hidden_class_id_selectors(classes, ids, exceptions);
        selectors_to_css(sel.iter())
    }
}

fn selectors_to_css<'a>(sel: impl Iterator<Item = &'a String>) -> String {
    let v: Vec<&String> = sel.collect();
    v.chunks(200).map(|c| format!("{}{{display:none!important}}\n", c.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(","))).collect()
}

/// Per-tab record of what was blocked.
#[derive(Default)]
pub struct Blocked {
    pub total: u32,
    pub by_host: BTreeMap<String, u32>,
}

impl Blocked {
    pub fn add(&mut self, host: String) {
        self.total += 1;
        *self.by_host.entry(host).or_default() += 1;
    }
}

/// Removes tracking parameters (utm_*, fbclid, gclid, …) from a URL. Returns None if nothing changed.
pub fn clean_url(url: &url::Url) -> Option<url::Url> {
    const EXACT: &[&str] = &[
        "fbclid", "gclid", "gclsrc", "dclid", "gbraid", "wbraid", "msclkid", "yclid", "twclid", "ttclid", "igshid", "igsh", "mc_cid", "mc_eid",
        "_hsenc", "_hsmi", "__hssc", "__hstc", "__hsfp", "hsCtaTracking", "mkt_tok", "vero_conv", "vero_id", "oly_anon_id", "oly_enc_id",
        "rb_clickid", "s_cid", "ef_id", "_openstat", "wickedid", "srsltid", "si", "ref_src", "ref_url", "spm", "scm",
    ];
    if url.query().is_none() {
        return None;
    }
    let host = url.host_str().unwrap_or("");
    let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let keep: Vec<&(String, String)> = pairs
        .iter()
        .filter(|(k, _)| {
            let kl = k.to_lowercase();
            // `si` is a tracker on YouTube/Spotify share links only.
            if kl == "si" && !(host.contains("youtu") || host.contains("spotify")) {
                return true;
            }
            !(kl.starts_with("utm_") || kl.starts_with("pk_") || kl.starts_with("mtm_") || EXACT.iter().any(|e| e.eq_ignore_ascii_case(&kl)))
        })
        .collect();
    if keep.len() == pairs.len() {
        return None;
    }
    let mut out = url.clone();
    if keep.is_empty() {
        out.set_query(None);
    } else {
        out.query_pairs_mut().clear().extend_pairs(keep.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    }
    Some(out)
}
