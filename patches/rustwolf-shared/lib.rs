//! Which site each RustWolf tab shows, shared by RustWolf's patched Servo crates (network and
//! storage) so they can keep sites apart:
//! - `localStorage` / `sessionStorage` of a site embedded in another site's page are partitioned:
//!   a tracker iframe gets a separate, empty storage on every site it's embedded in;
//! - script cookies (`document.cookie`, Cookie Store), IndexedDB and Cache Storage, whose engine
//!   messages don't say which tab they come from, are only available to sites open in a tab.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};

use net_traits::pub_domains::reg_suffix;
use servo_base::id::WebViewId;
use servo_url::{ImmutableOrigin, ServoUrl};

static PARTITION: AtomicBool = AtomicBool::new(true);
static TOP_LEVEL: LazyLock<RwLock<HashMap<WebViewId, String>>> = LazyLock::new(Default::default);

/// Turn site separation for storage and script cookies on or off.
pub fn set_partitioning(on: bool) {
    PARTITION.store(on, Ordering::Relaxed);
}

pub fn partitioning() -> bool {
    PARTITION.load(Ordering::Relaxed)
}

/// The registrable domain ("site") of a host, e.g. `news.example.co.uk` → `example.co.uk`.
pub fn site_of_host(host: &str) -> String {
    reg_suffix(host).to_ascii_lowercase()
}

pub fn site_of_url(url: &ServoUrl) -> Option<String> {
    url.host_str().map(site_of_host)
}

/// A tab navigated to `url`: that's now the tab's site.
pub fn note_top_level(webview: WebViewId, url: &ServoUrl) {
    if let (Some(site), Ok(mut map)) = (site_of_url(url), TOP_LEVEL.write()) {
        map.insert(webview, site);
    }
}

/// A tab was closed.
pub fn forget_tab(webview: WebViewId) {
    if let Ok(mut map) = TOP_LEVEL.write() {
        map.remove(&webview);
    }
}

pub fn top_site(webview: WebViewId) -> Option<String> {
    TOP_LEVEL.read().ok().and_then(|m| m.get(&webview).cloned())
}

fn site_open_in_a_tab(site: &str) -> bool {
    TOP_LEVEL.read().map(|m| m.values().any(|s| s == site)).unwrap_or(true)
}

/// May script on this URL use cookies? Only sites open in a tab can.
pub fn url_open_in_a_tab(url: &ServoUrl) -> bool {
    if !partitioning() {
        return true;
    }
    match site_of_url(url) {
        Some(site) => site_open_in_a_tab(&site),
        None => true, // no host (about:, data:, file:): nothing to track with
    }
}

/// May this origin use IndexedDB / Cache Storage? Only sites open in a tab can.
pub fn origin_open_in_a_tab(origin: &ImmutableOrigin) -> bool {
    if !partitioning() {
        return true;
    }
    match origin {
        ImmutableOrigin::Tuple(_, host, _) => site_open_in_a_tab(&site_of_host(&host.to_string())),
        ImmutableOrigin::Opaque(_) => true,
    }
}

/// Web storage key for `origin` in this tab: unchanged for the tab's own site, a separate
/// per-site "partition" for other sites embedded in its page.
pub fn partition_origin(webview: WebViewId, origin: ImmutableOrigin) -> ImmutableOrigin {
    if !partitioning() {
        return origin;
    }
    let ImmutableOrigin::Tuple(scheme, host, port) = &origin else { return origin };
    let host = host.to_string();
    match top_site(webview) {
        Some(top) if top != site_of_host(&host) => ImmutableOrigin::Tuple(
            scheme.clone(),
            url::Host::Domain(format!("{host}.{top}.partition.rustwolf.invalid")),
            *port,
        ),
        _ => origin,
    }
}
