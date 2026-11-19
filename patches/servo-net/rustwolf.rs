//! RustWolf additions to Servo's network code (MPL-2.0, like the rest of this crate).
//!
//! - Third-party cookie blocking: cookies are only sent to / accepted from the site shown in the
//!   tab (the top-level site). Requests to other sites embedded in the page (trackers, ad iframes)
//!   go out without cookies and can't set any.
//! - Firefox / Tor Browser identical headers, so RustWolf blends into that crowd.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};

use http::header::{self, HeaderValue};
use net_traits::pub_domains::reg_suffix;
use net_traits::request::{Destination, Request};
use servo_base::id::WebViewId;
use servo_url::ServoUrl;

static BLOCK_THIRD_PARTY_COOKIES: AtomicBool = AtomicBool::new(true);
static FIREFOX_HEADERS: AtomicBool = AtomicBool::new(true);
/// Top-level site of each tab, learned from its main-frame (document) requests.
static TOP_LEVEL: LazyLock<RwLock<HashMap<WebViewId, String>>> = LazyLock::new(Default::default);

pub fn set_block_third_party_cookies(on: bool) {
    BLOCK_THIRD_PARTY_COOKIES.store(on, Ordering::Relaxed);
}

pub fn set_firefox_headers(on: bool) {
    FIREFOX_HEADERS.store(on, Ordering::Relaxed);
}

fn site(url: &ServoUrl) -> Option<String> {
    url.host_str().map(|h| reg_suffix(h).to_ascii_lowercase())
}

/// True when this request goes to a different site than the one shown in its tab.
pub(crate) fn third_party(request: &Request, url: &ServoUrl) -> bool {
    if !BLOCK_THIRD_PARTY_COOKIES.load(Ordering::Relaxed) {
        return false;
    }
    let (Some(id), Some(s)) = (request.target_webview_id, site(url)) else { return false };
    if request.destination == Destination::Document {
        // A top-level navigation: this is now the tab's site.
        if let Ok(mut map) = TOP_LEVEL.write() {
            map.insert(id, s);
        }
        return false;
    }
    TOP_LEVEL.read().ok().and_then(|m| m.get(&id).cloned()).is_some_and(|top| top != s)
}

/// Same `Accept-Language` and image `Accept` values as Firefox with resist-fingerprinting
/// (Tor Browser / Mullvad Browser).
pub(crate) fn adjust_request_headers(request: &mut Request) {
    if !FIREFOX_HEADERS.load(Ordering::Relaxed) {
        return;
    }
    request.headers.insert(header::ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
    if request.destination == Destination::Image &&
        request.headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) == Some("image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5")
    {
        request.headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5"),
        );
    }
}
