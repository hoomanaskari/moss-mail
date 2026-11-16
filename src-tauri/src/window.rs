//! The red button hides Moss instead of quitting it, as Mail does: email keeps syncing and
//! sending in the background, and clicking Moss in the Dock brings the window back. ⌘Q quits.
//! ⌘W goes the same way as the red button.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, RunEvent, Runtime, WebviewWindow, WindowEvent};

/// Whether the red button keeps Moss running. Only on a Mac, where the Dock brings it back:
/// elsewhere a hidden window would leave no way back in.
pub const CLOSING_HIDES: bool = cfg!(target_os = "macos");

/// A window that can be put away and brought back.
pub trait Shown {
    fn is_full_screen(&self) -> bool;
    fn hide(&self);
    /// Hides all of Moss, as ⌘H does.
    fn hide_app(&self);
    fn show(&self);
}

// What can't be done leaves the window as it is; there is nothing more to do, so errors are
// dropped.
impl<R: Runtime> Shown for WebviewWindow<R> {
    fn is_full_screen(&self) -> bool {
        self.is_fullscreen().unwrap_or(false)
    }

    fn hide(&self) {
        let _ = WebviewWindow::hide(self);
    }

    fn hide_app(&self) {
        #[cfg(target_os = "macos")]
        let _ = self.app_handle().hide();
    }

    fn show(&self) {
        let _ = self.unminimize();
        let _ = WebviewWindow::show(self);
        let _ = self.set_focus();
    }
}

/// Hides every window when it is closed, if `hides`. (Not named "window": Tauri has a plugin of
/// that name and would quietly drop this one.) The Dock click that calls [`reopen`] is wired in
/// `run()`, as the test runtime can't send one.
pub fn plugin<R: Runtime>(hides: bool) -> TauriPlugin<R> {
    Builder::new("hide-on-close")
        .on_event(move |app, event| {
            if let RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. } = event {
                if app.get_webview_window(label).is_some_and(|window| close(&window, hides)) {
                    api.prevent_close();
                }
            }
        })
        .build()
}

/// Closing `window` hides it when `hides`; returns whether it should stay open. In full screen
/// it hides all of Moss instead: hiding just the window would leave an empty black screen, while
/// hiding Moss takes you back to where you were, and the Dock brings it back full screen.
pub fn close(window: &impl Shown, hides: bool) -> bool {
    if hides && window.is_full_screen() {
        window.hide_app();
    } else if hides {
        window.hide();
    }
    hides
}

/// Brings the main window back, out of the Dock if it was minimised.
pub fn reopen(window: Option<&impl Shown>) {
    if let Some(window) = window {
        window.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[derive(Default)]
    struct Fake {
        full_screen: bool,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Shown for Fake {
        fn is_full_screen(&self) -> bool {
            self.full_screen
        }
        fn hide(&self) {
            self.calls.borrow_mut().push("hide");
        }
        fn hide_app(&self) {
            self.calls.borrow_mut().push("hide app");
        }
        fn show(&self) {
            self.calls.borrow_mut().push("show");
        }
    }

    #[test]
    fn closing_hides_the_window_and_keeps_moss_running() {
        let window = Fake::default();
        assert!(close(&window, true));
        assert_eq!(*window.calls.borrow(), ["hide"]);
    }

    #[test]
    fn in_full_screen_closing_hides_all_of_moss() {
        // Hiding just the window would leave an empty black screen behind.
        let window = Fake { full_screen: true, ..Fake::default() };
        assert!(close(&window, true));
        assert_eq!(*window.calls.borrow(), ["hide app"]);
    }

    #[test]
    fn where_closing_doesnt_hide_the_window_closes() {
        let window = Fake::default();
        assert!(!close(&window, false));
        assert!(window.calls.borrow().is_empty());
    }

    #[test]
    fn only_a_mac_hides_moss_on_close() {
        assert_eq!(CLOSING_HIDES, cfg!(target_os = "macos"));
    }

    /// Runs Moss with one window and clicks its red button, then destroys the window so the run
    /// ends either way. Returns how many windows the close left, or None if it quit Moss.
    fn windows_after_close(hides: bool) -> Option<usize> {
        let app = mock_builder().plugin(plugin(hides)).build(mock_context(noop_assets())).unwrap();
        tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let left = Arc::new(Mutex::new(None));
        let after = left.clone();
        app.run(move |app, event| match event {
            // Before the loop runs, the mock runtime drops a closed window without asking.
            RunEvent::Ready => {
                let window = app.get_webview_window("main").unwrap();
                window.close().unwrap();
                window.destroy().unwrap();
            }
            RunEvent::MainEventsCleared => {
                after.lock().unwrap().get_or_insert(app.webview_windows().len());
            }
            _ => {}
        });
        let left = *left.lock().unwrap();
        left
    }

    #[test]
    fn the_red_button_keeps_the_window_for_the_dock_to_bring_back() {
        assert_eq!(windows_after_close(true), Some(1));
    }

    #[test]
    fn where_closing_doesnt_hide_the_red_button_quits() {
        assert_eq!(windows_after_close(false), None);
    }

    #[test]
    fn reopening_shows_the_window() {
        let window = Fake::default();
        reopen(Some(&window));
        assert_eq!(*window.calls.borrow(), ["show"]);
        // No window: nothing to bring back, and nothing goes wrong.
        reopen(None::<&Fake>);
    }

    #[test]
    fn a_real_window_hides_and_shows() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let window =
            tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        // The mock runtime always reports a window as visible, so this checks only that hiding
        // and showing go through; the Fake tests check which is called when.
        assert!(!Shown::is_full_screen(&window));
        assert!(close(&window, true));
        Shown::hide_app(&window);
        reopen(Some(&window));
        assert!(window.is_visible().unwrap());
    }
}
