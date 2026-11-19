//! Mail links (`mailto:`) other apps send Moss once it is your default email app, as macOS's
//! "open this URL" event. `Info.plist` says Moss can open them. A link can arrive before the
//! window has loaded (it may be what started Moss), so each one waits here until the window
//! takes it, and the window is told when one arrives. The window reads the link
//! (`src/lib/mailLink.ts`); here only its scheme is checked.

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, Url};

/// What the window hears when a link arrives (`src/services/mailLinks.ts` has the same name).
pub const EVENT: &str = "moss://mail-link";

/// Links waiting for the window.
#[derive(Default)]
pub struct MailLinks(Mutex<Vec<String>>);

/// Keeps the mail links among `urls`, brings the window forward and tells it.
pub fn arrived<R: Runtime>(app: &AppHandle<R>, urls: &[Url]) {
    let links: Vec<String> = urls
        .iter()
        .filter(|u| u.scheme() == "mailto")
        .map(Url::to_string)
        .collect();
    if links.is_empty() {
        return;
    }
    app.state::<MailLinks>().0.lock().unwrap().extend(links);
    crate::window::reopen(app.get_webview_window("main").as_ref());
    let _ = app.emit(EVENT, ());
}

/// The links waiting for the window, which it now has.
#[tauri::command]
pub fn take_mail_links(links: tauri::State<'_, MailLinks>) -> Vec<String> {
    std::mem::take(&mut *links.0.lock().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri::Listener;

    fn app() -> (tauri::App<MockRuntime>, Arc<Mutex<usize>>) {
        let app = mock_builder()
            .manage(MailLinks::default())
            .build(mock_context(noop_assets()))
            .unwrap();
        let heard = Arc::new(Mutex::new(0));
        let sink = heard.clone();
        app.listen(EVENT, move |_| *sink.lock().unwrap() += 1);
        (app, heard)
    }

    fn urls(list: &[&str]) -> Vec<Url> {
        list.iter().map(|u| Url::parse(u).unwrap()).collect()
    }

    #[test]
    fn macos_is_told_moss_opens_mail_links() {
        // Without this Moss isn't offered as the default email app, and no link reaches it.
        let plist = include_str!("../Info.plist");
        let types = &plist[plist
            .find("<key>CFBundleURLTypes</key>")
            .expect("Info.plist lists the links Moss opens")..];
        let schemes = &types[types.find("<key>CFBundleURLSchemes</key>").unwrap()..];
        let list = &schemes[..schemes.find("</array>").unwrap()];
        assert!(list.contains("<string>mailto</string>"), "{list}");
        assert!(
            types.contains("<key>CFBundleTypeRole</key>\n\t\t\t<string>Viewer</string>"),
            "Moss opens them"
        );
    }

    #[test]
    fn the_window_listens_for_this_name() {
        assert_eq!(EVENT, "moss://mail-link");
    }

    #[test]
    fn a_mail_link_waits_for_the_window_which_is_told() {
        let (app, heard) = app();
        // The test runtime keeps no visibility, so `window::reopen`'s own tests check that part.
        let _window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        arrived(
            app.handle(),
            &urls(&[
                "mailto:lena@northwind.co?subject=Q3%20plan",
                "mailto:jonas@northwind.co",
            ]),
        );
        assert_eq!(*heard.lock().unwrap(), 1);
        let links = app.state::<MailLinks>();
        assert_eq!(
            take_mail_links(links.clone()),
            [
                "mailto:lena@northwind.co?subject=Q3%20plan",
                "mailto:jonas@northwind.co"
            ]
        );
        assert!(take_mail_links(links).is_empty(), "each link opens once");
    }

    #[test]
    fn a_link_that_arrives_with_no_window_still_waits() {
        let (app, heard) = app();
        arrived(app.handle(), &urls(&["mailto:lena@northwind.co"]));
        assert_eq!(*heard.lock().unwrap(), 1);
        assert_eq!(
            take_mail_links(app.state::<MailLinks>()),
            ["mailto:lena@northwind.co"]
        );
    }

    #[test]
    fn other_links_are_not_kept_and_nobody_is_told() {
        let (app, heard) = app();
        arrived(
            app.handle(),
            &urls(&["https://northwind.co", "file:///etc/passwd", "moss://x"]),
        );
        assert_eq!(*heard.lock().unwrap(), 0);
        assert!(take_mail_links(app.state::<MailLinks>()).is_empty());
    }
}
