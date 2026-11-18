//! The user's choices from the first-run setup (search engine, protections, privacy tools).
//! Saved to %APPDATA%\RustWolf\settings.json: only these choices, never browsing data.

use serde::{Deserialize, Serialize};

pub struct SearchEngine {
    pub id: &'static str,
    pub name: &'static str,
    /// `{}` is replaced by the URL-encoded search terms.
    pub url: &'static str,
    pub blurb: &'static str,
    pub color: [u8; 3],
}

pub const ENGINES: &[SearchEngine] = &[
    SearchEngine { id: "duckduckgo", name: "DuckDuckGo", url: "https://duckduckgo.com/?q={}", blurb: "No tracking, no search history. Popular and fast.", color: [222, 88, 51] },
    SearchEngine { id: "brave", name: "Brave Search", url: "https://search.brave.com/search?q={}", blurb: "Its own independent index. No profiling.", color: [251, 84, 43] },
    SearchEngine { id: "mojeek", name: "Mojeek", url: "https://www.mojeek.com/search?q={}", blurb: "Own crawler and index, no tracking. Made in the UK.", color: [90, 160, 110] },
    SearchEngine { id: "qwant", name: "Qwant", url: "https://www.qwant.com/?q={}", blurb: "European search engine that doesn't track you.", color: [60, 140, 230] },
    SearchEngine { id: "kagi", name: "Kagi", url: "https://kagi.com/search?q={}", blurb: "Paid, ad-free results. Needs a Kagi account.", color: [255, 182, 60] },
];

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub setup_done: bool,
    pub search: String,
    // Protection
    pub block_ads: bool,
    pub cookie_banners: bool,
    pub annoyances: bool,
    pub social: bool,
    pub malware: bool,
    pub fingerprint: bool,
    pub clean_links: bool,
    pub https_only: bool,
    /// No JIT / WebAssembly: closes the most-attacked part of the JavaScript engine.
    pub hardened: bool,
    /// Sites where third-party script files are blocked.
    pub no_3p_scripts: Vec<String>,
    // Privacy front-ends
    pub redirect_youtube: bool,
    pub redirect_reddit: bool,
    pub redirect_twitter: bool,
    pub redirect_medium: bool,
    // Network
    pub tor: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self::recommended()
    }
}

impl Settings {
    pub fn recommended() -> Self {
        Self {
            setup_done: false,
            search: "duckduckgo".into(),
            block_ads: true,
            cookie_banners: true,
            annoyances: true,
            social: false,
            malware: true,
            fingerprint: true,
            clean_links: true,
            https_only: true,
            hardened: false,
            no_3p_scripts: Vec::new(),
            redirect_youtube: false,
            redirect_reddit: false,
            redirect_twitter: false,
            redirect_medium: false,
            tor: true, // on by default: every site sees a Tor exit IP, not yours
        }
    }

    /// Recommended + everything that can block more (may break a few more sites).
    pub fn strict_protection(&mut self) {
        self.block_ads = true;
        self.cookie_banners = true;
        self.annoyances = true;
        self.social = true;
        self.malware = true;
        self.fingerprint = true;
        self.clean_links = true;
        self.https_only = true;
        self.hardened = true;
    }

    pub fn recommended_protection(&mut self) {
        let r = Self::recommended();
        self.block_ads = r.block_ads;
        self.cookie_banners = r.cookie_banners;
        self.annoyances = r.annoyances;
        self.social = r.social;
        self.malware = r.malware;
        self.fingerprint = r.fingerprint;
        self.clean_links = r.clean_links;
        self.https_only = r.https_only;
        self.hardened = r.hardened;
    }

    pub fn is_strict(&self) -> bool {
        self.block_ads && self.cookie_banners && self.annoyances && self.social && self.malware && self.fingerprint && self.clean_links && self.https_only && self.hardened
    }

    pub fn engine(&self) -> &'static SearchEngine {
        ENGINES.iter().find(|e| e.id == self.search).unwrap_or(&ENGINES[0])
    }

    pub fn search_url(&self, query: &str) -> Option<url::Url> {
        let enc: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        url::Url::parse(&self.engine().url.replace("{}", &enc)).ok()
    }

    fn path() -> Option<std::path::PathBuf> {
        crate::data_dir().map(|d| d.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    /// Privacy front-ends: the same page through a private, tracker-free interface.
    pub fn redirect(&self, url: &url::Url) -> Option<url::Url> {
        let host = url.host_str()?.to_lowercase();
        let host = host.trim_start_matches("www.").trim_start_matches("m.");
        let path_q = |u: &url::Url| format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default());
        let to = |base: &str, rest: String| url::Url::parse(&format!("{base}{rest}")).ok();
        if self.redirect_youtube && (host == "youtube.com" || host == "music.youtube.com") {
            return to("https://yewtu.be", path_q(url));
        }
        if self.redirect_youtube && host == "youtu.be" {
            return to("https://yewtu.be", format!("/watch?v={}", url.path().trim_start_matches('/')));
        }
        if self.redirect_reddit && (host == "reddit.com" || host == "old.reddit.com" || host == "new.reddit.com") {
            return to("https://safereddit.com", path_q(url));
        }
        None
    }
}
