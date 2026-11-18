//! Built-in ad & tracker blocking, using Brave's Rust ad-block engine (`adblock` crate, MPL-2.0).

use std::collections::BTreeMap;

pub struct Blocker {
    engine: adblock::Engine,
    pub rule_count: usize,
}

impl Blocker {
    pub fn new() -> Self {
        let mut rules = include_str!("../assets/filters.txt").to_string();
        if let Some(extra) = crate::data_dir().map(|d| d.join("filters.txt")) {
            if let Ok(text) = std::fs::read_to_string(extra) {
                rules.push('\n');
                rules.push_str(&text);
            }
        }
        let rule_count = rules.lines().filter(|l| { let l = l.trim(); !l.is_empty() && !l.starts_with('!') }).count();
        Self { engine: adblock::Engine::new_with_list_text(rules), rule_count }
    }

    /// Should this request be blocked? `page` is the top-level page URL.
    pub fn should_block(&self, url: &str, page: &str, destination: &str) -> bool {
        match adblock::request::Request::new(url, page, destination, "GET") {
            Ok(req) => self.engine.check_network_request(&req).should_block(),
            Err(_) => false,
        }
    }
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
