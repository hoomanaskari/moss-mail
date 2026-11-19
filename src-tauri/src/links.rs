//! Keeps Moss's windows on Moss's own pages.
//!
//! Links clicked in an email do not come here: the email frame is sandboxed with
//! `allow-scripts`, so Moss's click listener on it handles them itself (`src/lib/links.ts`), and
//! the app CSP's `frame-src 'self'` blocks any web page a frame tries to load before WebKit asks
//! Moss. What still passes through here is every other navigation of a window: Moss's own
//! pages load, web links go to the default browser instead of replacing the app, and mail links
//! open a new email in Moss, as they do from other apps.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};
use tauri_plugin_opener::open_url;

/// Sends every navigation in every window through [`navigate`]: web links open with the
/// system's default browser, mail links in Moss.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("links")
        .on_navigation(|webview, url| {
            navigate(url, open_url, |mail| {
                crate::mail_links::arrived(webview.app_handle(), std::slice::from_ref(mail))
            })
        })
        .build()
}

/// Whether `url` may load in Moss. Web links are handed to `open` (with no particular app)
/// instead; if that fails there is nothing more to do, so its error is dropped. Mail links go
/// to `mail`.
pub fn navigate<'a, E>(
    url: &'a Url,
    open: impl FnOnce(&'a str, Option<&'a str>) -> Result<(), E>,
    mail: impl FnOnce(&'a Url),
) -> bool {
    match url.scheme() {
        "mailto" => mail(url),
        // Moss's own pages: tauri://localhost, http://tauri.localhost and the dev server.
        "http" | "https"
            if !url
                .host_str()
                .is_some_and(|h| h == "localhost" || h.ends_with(".localhost")) =>
        {
            let _ = open(url.as_str(), None);
        }
        _ => return true,
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::plugin::Plugin;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    /// Where `url` went: the browser, a new email in Moss, or (both empty) a page of Moss.
    fn went(url: &str) -> (Option<String>, Option<String>) {
        let (mut opened, mut mailed) = (None, None);
        let allowed = navigate(
            &Url::parse(url).unwrap(),
            |u, with| {
                assert_eq!(with, None, "the default app opens it");
                opened = Some(u.to_string());
                Err("no browser in tests")
            },
            |u| mailed = Some(u.to_string()),
        );
        assert_eq!(
            allowed,
            opened.is_none() && mailed.is_none(),
            "{url}: a link either loads in Moss or goes somewhere else"
        );
        (opened, mailed)
    }

    #[test]
    fn web_links_open_in_the_browser() {
        for url in ["https://c.gle/AGSJxQxJ", "http://example.com/a?b=1#c"] {
            assert_eq!(
                went(url),
                (Some(Url::parse(url).unwrap().to_string()), None)
            );
        }
    }

    #[test]
    fn mail_links_open_a_new_email_in_moss() {
        let url = "mailto:lena@northwind.co?subject=Hi";
        assert_eq!(went(url), (None, Some(url.to_string())));
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
            assert_eq!(went(url), (None, None), "{url}");
        }
    }

    #[test]
    fn the_plugin_checks_every_navigation() {
        let app = mock_builder()
            .manage(crate::mail_links::MailLinks::default())
            .build(mock_context(noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let mut links = plugin();
        let own = Url::parse("tauri://localhost/").unwrap();
        assert!(links.on_navigation(window.as_ref(), &own));
        let mail = Url::parse("mailto:lena@northwind.co").unwrap();
        assert!(!links.on_navigation(window.as_ref(), &mail));
        let waiting = crate::mail_links::take_mail_links(app.state());
        assert_eq!(
            waiting,
            ["mailto:lena@northwind.co"],
            "it waits for the window"
        );
    }

    #[test]
    fn other_kinds_of_address_are_not_handed_to_the_browser() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://x.example/",
        ] {
            assert_eq!(went(url), (None, None), "{url}");
        }
    }
}
