//! Keeps Moss's windows on Moss's own pages.
//!
//! Links clicked in an email do not come here: the email frame is sandboxed with
//! `allow-scripts`, so Moss's click listener on it sends them to the browser itself, and the
//! app CSP's `frame-src 'self'` blocks any web page a frame tries to load before WebKit asks
//! Moss. What still passes through here is every other navigation of a window: Moss's own
//! pages load, and web and mail links go to the default browser instead of replacing the app.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Runtime, Url};
use tauri_plugin_opener::open_url;

/// Sends every navigation in every window through [`navigate`], opening outside links with the
/// system's default app.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("links")
        .on_navigation(|_, url| navigate(url, open_url))
        .build()
}

/// Whether `url` may load in Moss. Web and mail links are handed to `open` (with no particular
/// app) instead; if that fails there is nothing more to do, so its error is dropped.
pub fn navigate<'a, E>(
    url: &'a Url,
    open: impl FnOnce(&'a str, Option<&'a str>) -> Result<(), E>,
) -> bool {
    let outside = match url.scheme() {
        "mailto" => true,
        // Moss's own pages: tauri://localhost, http://tauri.localhost and the dev server.
        "http" | "https" => !url
            .host_str()
            .is_some_and(|h| h == "localhost" || h.ends_with(".localhost")),
        _ => false,
    };
    if outside {
        let _ = open(url.as_str(), None);
    }
    !outside
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::plugin::Plugin;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn opened(url: &str) -> Option<String> {
        let mut opened = None;
        let allowed = navigate(&Url::parse(url).unwrap(), |u, with| {
            assert_eq!(with, None, "the default app opens it");
            opened = Some(u.to_string());
            Err("no browser in tests")
        });
        assert_eq!(
            allowed,
            opened.is_none(),
            "{url}: a link either loads in Moss or opens outside"
        );
        opened
    }

    #[test]
    fn web_and_mail_links_open_in_the_browser() {
        for url in [
            "https://c.gle/AGSJxQxJ",
            "http://example.com/a?b=1#c",
            "mailto:lena@northwind.co",
        ] {
            assert_eq!(
                opened(url).as_deref(),
                Some(Url::parse(url).unwrap().as_str())
            );
        }
    }

    #[test]
    fn moss_own_pages_load_in_moss() {
        for url in [
            "tauri://localhost/",
            "http://tauri.localhost/index.html",
            "http://localhost:1420/",
            "about:srcdoc",
            "about:blank",
        ] {
            assert_eq!(opened(url), None, "{url}");
        }
    }

    #[test]
    fn the_plugin_checks_every_navigation() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let mut links = plugin();
        let own = Url::parse("tauri://localhost/").unwrap();
        assert!(links.on_navigation(window.as_ref(), &own));
    }

    #[test]
    fn other_kinds_of_address_are_not_handed_to_the_browser() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://x.example/",
        ] {
            assert_eq!(opened(url), None, "{url}");
        }
    }
}
