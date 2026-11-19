//! RustWolf additions to Servo's network code (MPL-2.0, like the rest of this crate).
//!
//! - Third-party cookie blocking: cookies are only sent to / accepted from the site shown in the
//!   tab (the top-level site). Requests to other sites embedded in the page (trackers, ad iframes)
//!   go out without cookies and can't set any.
//! - Script cookies (`document.cookie`, Cookie Store) only for sites open in a tab, so an embedded
//!   tracker can't use them either (see the `rustwolf-shared` crate).
//! - Firefox / Tor Browser identical headers, so RustWolf blends into that crowd.

use std::sync::atomic::{AtomicBool, Ordering};

use http::header::{self, HeaderValue};
use net_traits::request::{Destination, Request};
use net_traits::CookieSource;
use servo_url::ServoUrl;

static BLOCK_THIRD_PARTY_COOKIES: AtomicBool = AtomicBool::new(true);
static FIREFOX_HEADERS: AtomicBool = AtomicBool::new(true);

pub fn set_block_third_party_cookies(on: bool) {
    BLOCK_THIRD_PARTY_COOKIES.store(on, Ordering::Relaxed);
}

pub fn set_firefox_headers(on: bool) {
    FIREFOX_HEADERS.store(on, Ordering::Relaxed);
}

/// A top-level navigation (main frame of a tab): remember that tab's site.
pub(crate) fn note_navigation(request: &Request, url: &ServoUrl) {
    if let (Destination::Document, Some(id)) = (request.destination, request.target_webview_id) {
        rustwolf_shared::note_top_level(id, url);
    }
}

/// True when this request goes to a different site than the one shown in its tab.
pub(crate) fn third_party(request: &Request, url: &ServoUrl) -> bool {
    note_navigation(request, url);
    if !BLOCK_THIRD_PARTY_COOKIES.load(Ordering::Relaxed) || request.destination == Destination::Document {
        return false;
    }
    let (Some(id), Some(s)) = (request.target_webview_id, rustwolf_shared::site_of_url(url)) else { return false };
    rustwolf_shared::top_site(id).is_some_and(|top| top != s)
}

/// May a page's script read / write cookies for this URL? Not when the site isn't open in any tab
/// (it's only embedded in other sites' pages: a tracker).
pub(crate) fn script_cookies_allowed(url: &ServoUrl, source: CookieSource) -> bool {
    source == CookieSource::HTTP || rustwolf_shared::url_open_in_a_tab(url)
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
