//! Moss opens where you left it, as Mac apps do with AppKit's frame autosave. The window's
//! frame is saved on every move and resize, when the red button hides it, and on quitting. On
//! launch it comes back at that size and place, shrunk to fit and kept on screen if the screen
//! it was on is smaller or gone, and maximized or in full screen if it was.
//!
//! Frames are in points (logical pixels), as AppKit keeps them: screens with different scale
//! factors share one space of points, but not one of physical pixels.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, PhysicalRect, RunEvent, Runtime,
    Window, WindowEvent,
};

/// The smallest the window can be, as `minWidth` and `minHeight` in `tauri.conf.json`.
pub const MIN_SIZE: (f64, f64) = (960.0, 640.0);

/// How far in from each edge of its screen a maximized window that would fill it is placed
/// before it maximizes (see [`restore`]).
const UNZOOMED_INSET: f64 = 10.0;

/// A rectangle in points, from the top left of the main screen, y going down.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What Moss saves about its window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    /// The window's frame when it was last neither maximized nor full screen.
    pub frame: Rect,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub full_screen: bool,
}

/// Where a window saved at `saved` opens, given the usable area (inside the menu bar and Dock)
/// of each screen connected now, the main screen first.
///
/// It stays on the screen that holds most of it, or goes to the middle of the main screen if
/// none does, as when the screen it was on isn't connected. If it is bigger than that screen it
/// shrinks to fit, though never below `min`, and it moves just far enough to be fully on it.
pub fn fit(saved: Rect, screens: &[Rect], min: (f64, f64)) -> Rect {
    let most = screens
        .iter()
        .map(|screen| (overlap(&saved, screen), screen))
        .filter(|(area, _)| *area > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, screen)| screen);
    let Some(screen) = most.or(screens.first()) else { return saved };
    let width = saved.width.min(screen.width).max(min.0);
    let height = saved.height.min(screen.height).max(min.1);
    let (x, y) = if most.is_some() {
        (saved.x, saved.y)
    } else {
        (screen.x + (screen.width - width) / 2.0, screen.y + (screen.height - height) / 2.0)
    };
    let x = within(x, screen.x, screen.width - width);
    Rect { x, y: within(y, screen.y, screen.height - height), width, height }
}

/// How much of `a` and `b` overlap, in square points.
fn overlap(a: &Rect, b: &Rect) -> f64 {
    let width = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let height = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    width.max(0.0) * height.max(0.0)
}

/// `at`, moved into `start..=start + room`; `start` when there's no room.
fn within(at: f64, start: f64, room: f64) -> f64 {
    at.min(start + room).max(start)
}

/// A window whose frame can be read and set.
pub trait Framed {
    /// Its frame now, if it can tell.
    fn frame(&self) -> Option<Rect>;
    fn zoomed(&self) -> bool;
    fn in_full_screen(&self) -> bool;
    /// The usable area of each screen, the main screen first.
    fn screens(&self) -> Vec<Rect>;
    fn place(&self, frame: Rect);
    fn zoom(&self);
    fn enter_full_screen(&self);
}

/// Puts `window` back as it was saved: where it was, fitted to the screens there are now, then
/// full screen or maximized on that screen if it was. With nothing saved it stays as it opened.
pub fn restore(window: &impl Framed, saved: Option<Saved>) {
    let Some(saved) = saved else { return };
    let screens = window.screens();
    let mut frame = fit(saved.frame, &screens, MIN_SIZE);
    // AppKit takes a window that fills its screen as maximized already, and maximizing it would
    // put it back to a smaller size instead. Coming in a little from each edge, as that smaller
    // size does, lets it maximize; it also leaves a size to go back to.
    if saved.maximized && screens.contains(&frame) {
        frame = Rect {
            x: frame.x + UNZOOMED_INSET,
            y: frame.y + UNZOOMED_INSET,
            width: frame.width - 2.0 * UNZOOMED_INSET,
            height: frame.height - 2.0 * UNZOOMED_INSET,
        };
    }
    window.place(frame);
    if saved.full_screen {
        window.enter_full_screen();
    } else if saved.maximized {
        window.zoom();
    }
}

/// What to save about `window` now, given what was saved before. A maximized or full-screen
/// window keeps its earlier frame, so it comes back to that size when you take it out.
pub fn note(window: &impl Framed, before: Option<Saved>) -> Option<Saved> {
    let Some(now) = window.frame() else { return before };
    let maximized = window.zoomed();
    let full_screen = window.in_full_screen();
    let frame = match before {
        Some(before) if maximized || full_screen => before.frame,
        _ => now,
    };
    Some(Saved { frame, maximized, full_screen })
}

/// What was saved in `file`, if it can be read.
pub fn load(file: &Path) -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

/// Saves `saved` in `file`. If it can't be, Moss opens at its usual size next time.
pub fn save(file: &Path, saved: &Saved) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::to_vec(saved).unwrap_or_default());
}

/// The main window's frame as last saved, and the file it is saved in.
pub struct Frames {
    file: Option<PathBuf>,
    last: Mutex<Option<Saved>>,
}

impl Frames {
    /// Reads what `file` holds. Without a file nothing is saved.
    pub fn open(file: Option<PathBuf>) -> Self {
        let last = file.as_deref().and_then(load);
        Frames { file, last: Mutex::new(last) }
    }

    pub fn saved(&self) -> Option<Saved> {
        *self.last.lock().unwrap()
    }

    /// Saves `window`'s frame, if it changed. With no window there is nothing to save.
    pub fn note(&self, window: Option<&impl Framed>) {
        let Some(window) = window else { return };
        let mut last = self.last.lock().unwrap();
        let now = note(window, *last);
        if now != *last {
            *last = now;
            drop(last);
            self.write();
        }
    }

    /// Writes the last frame to the file.
    pub fn write(&self) {
        if let (Some(file), Some(saved)) = (&self.file, self.saved()) {
            save(file, &saved);
        }
    }
}

/// The main window comes back where it was, and its frame is saved in the file `file` names.
pub fn plugin<R: Runtime>(
    file: impl FnOnce(&AppHandle<R>) -> Option<PathBuf> + Send + 'static,
) -> TauriPlugin<R> {
    Builder::new("window-frame")
        .setup(|app, _| {
            app.manage(Frames::open(file(app)));
            Ok(())
        })
        .on_window_ready(|window| {
            if window.label() == MAIN {
                restore(&window, window.state::<Frames>().saved());
            }
        })
        .on_event(|app, event| {
            if let RunEvent::WindowEvent { label, event, .. } = event {
                if label == MAIN && saves(event) {
                    let window = app.get_webview_window(MAIN).map(|w| w.as_ref().window());
                    app.state::<Frames>().note(window.as_ref());
                }
            } else if let RunEvent::Exit = event {
                app.state::<Frames>().write();
            }
        })
        .build()
}

/// The window whose frame Moss keeps.
const MAIN: &str = "main";

/// Whether the window's frame is saved after `event`. The red button only hides the window, so
/// its close is saved too.
fn saves(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::CloseRequested { .. }
    )
}

/// `area` in points, at `scale` pixels to the point.
fn points(area: &PhysicalRect<i32, u32>, scale: f64) -> Rect {
    let at = area.position.to_logical::<f64>(scale);
    let size = area.size.to_logical::<f64>(scale);
    Rect { x: at.x, y: at.y, width: size.width, height: size.height }
}

/// The usable area of each of `monitors` in points, `main` first.
fn screens(mut monitors: Vec<Monitor>, main: Option<Monitor>) -> Vec<Rect> {
    let main = main.map(|m| *m.position());
    monitors.sort_by_key(|m| Some(*m.position()) != main);
    monitors.iter().map(|m| points(m.work_area(), m.scale_factor())).collect()
}

// What can't be done leaves the window as it is, so errors are dropped.
impl<R: Runtime> Framed for Window<R> {
    fn frame(&self) -> Option<Rect> {
        let scale = self.scale_factor().ok()?;
        let at = self.outer_position().ok()?.to_logical::<f64>(scale);
        let size = self.inner_size().ok()?.to_logical::<f64>(scale);
        Some(Rect { x: at.x, y: at.y, width: size.width, height: size.height })
    }

    fn zoomed(&self) -> bool {
        self.is_maximized().unwrap_or(false)
    }

    fn in_full_screen(&self) -> bool {
        self.is_fullscreen().unwrap_or(false)
    }

    fn screens(&self) -> Vec<Rect> {
        let main = self.primary_monitor().ok().flatten();
        screens(self.available_monitors().unwrap_or_default(), main)
    }

    fn place(&self, frame: Rect) {
        // Size first: AppKit resizes from the bottom left, which would move the top.
        let _ = self.set_size(LogicalSize::new(frame.width, frame.height));
        let _ = self.set_position(LogicalPosition::new(frame.x, frame.y));
    }

    fn zoom(&self) {
        let _ = self.maximize();
    }

    fn enter_full_screen(&self) {
        let _ = self.set_fullscreen(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    /// A MacBook's screen below its menu bar and above the Dock.
    const LAPTOP: Rect = Rect { x: 0.0, y: 33.0, width: 1512.0, height: 870.0 };
    /// A 27-inch display to the left of the laptop, its top a little higher.
    const BIG: Rect = Rect { x: -2560.0, y: -300.0, width: 2560.0, height: 1415.0 };

    #[test]
    fn a_window_that_fits_its_screen_opens_where_it_was() {
        let cases = [
            ("on the laptop", rect(100.0, 80.0, 1200.0, 700.0), vec![LAPTOP]),
            ("on the big display", rect(-2400.0, -200.0, 1800.0, 1200.0), vec![LAPTOP, BIG]),
            ("filling the laptop exactly", LAPTOP, vec![LAPTOP]),
        ];
        for (name, saved, screens) in cases {
            assert_eq!(fit(saved, &screens, MIN_SIZE), saved, "{name}");
        }
    }

    #[test]
    fn a_window_too_big_for_its_screen_shrinks_to_fit_on_it() {
        let cases = [
            // Saved on the big display, which is now gone: shrunk onto the laptop.
            ("big display gone", rect(-2400.0, -200.0, 1800.0, 1200.0), vec![LAPTOP], LAPTOP),
            // Wider than the laptop but not taller: only the width shrinks.
            (
                "too wide",
                rect(50.0, 100.0, 1700.0, 700.0),
                vec![LAPTOP],
                rect(0.0, 100.0, 1512.0, 700.0),
            ),
            // Taller but not wider: only the height shrinks, and it moves up to fit.
            (
                "too tall",
                rect(200.0, 300.0, 1000.0, 1000.0),
                vec![LAPTOP],
                rect(200.0, 33.0, 1000.0, 870.0),
            ),
        ];
        for (name, saved, screens, opens) in cases {
            assert_eq!(fit(saved, &screens, MIN_SIZE), opens, "{name}");
        }
    }

    #[test]
    fn a_window_partly_off_its_screen_moves_back_on() {
        let cases = [
            ("under the menu bar", rect(100.0, 0.0, 1000.0, 700.0), rect(100.0, 33.0, 1000.0, 700.0)),
            ("past the right edge", rect(900.0, 80.0, 1000.0, 700.0), rect(512.0, 80.0, 1000.0, 700.0)),
            ("past the left edge", rect(-50.0, 80.0, 1000.0, 700.0), rect(0.0, 80.0, 1000.0, 700.0)),
            ("behind the Dock", rect(100.0, 500.0, 1000.0, 700.0), rect(100.0, 203.0, 1000.0, 700.0)),
        ];
        for (name, saved, opens) in cases {
            assert_eq!(fit(saved, &[LAPTOP], MIN_SIZE), opens, "{name}");
        }
    }

    #[test]
    fn a_window_across_two_screens_goes_to_the_one_with_most_of_it() {
        // Mostly on the laptop, a little on the big display.
        let saved = rect(-200.0, 80.0, 1000.0, 700.0);
        assert_eq!(fit(saved, &[LAPTOP, BIG], MIN_SIZE), rect(0.0, 80.0, 1000.0, 700.0));
        // Mostly on the big display.
        let saved = rect(-800.0, 80.0, 1000.0, 700.0);
        assert_eq!(fit(saved, &[LAPTOP, BIG], MIN_SIZE), rect(-1000.0, 80.0, 1000.0, 700.0));
    }

    #[test]
    fn a_window_on_no_screen_opens_in_the_middle_of_the_main_one() {
        // The big display was to the right; now only the laptop and one on the left are there.
        let saved = rect(2000.0, 100.0, 1000.0, 700.0);
        assert_eq!(fit(saved, &[LAPTOP, BIG], MIN_SIZE), rect(256.0, 118.0, 1000.0, 700.0));
        // Bigger than the main screen as well: it fills it.
        let saved = rect(2000.0, 100.0, 2000.0, 1200.0);
        assert_eq!(fit(saved, &[LAPTOP, BIG], MIN_SIZE), LAPTOP);
    }

    #[test]
    fn a_window_never_shrinks_below_its_smallest_size() {
        // A screen narrower than the window can be: as small as allowed, from its top left.
        let small = rect(0.0, 25.0, 800.0, 575.0);
        assert_eq!(fit(rect(10.0, 40.0, 1200.0, 800.0), &[small], MIN_SIZE), rect(0.0, 25.0, 960.0, 640.0));
        // A saved size below the smallest grows to it.
        assert_eq!(fit(rect(100.0, 80.0, 300.0, 200.0), &[LAPTOP], MIN_SIZE), rect(100.0, 80.0, 960.0, 640.0));
    }

    #[test]
    fn with_no_screens_to_go_by_the_window_opens_where_it_was() {
        let saved = rect(-2400.0, -200.0, 1800.0, 1200.0);
        assert_eq!(fit(saved, &[], MIN_SIZE), saved);
    }

    /// A window that records what is done to it.
    #[derive(Default)]
    struct Fake {
        frame: Option<Rect>,
        zoomed: bool,
        full_screen: bool,
        screens: Vec<Rect>,
        calls: RefCell<Vec<String>>,
    }

    impl Framed for Fake {
        fn frame(&self) -> Option<Rect> {
            self.frame
        }
        fn zoomed(&self) -> bool {
            self.zoomed
        }
        fn in_full_screen(&self) -> bool {
            self.full_screen
        }
        fn screens(&self) -> Vec<Rect> {
            self.screens.clone()
        }
        fn place(&self, f: Rect) {
            self.calls.borrow_mut().push(format!("place {} {} {} {}", f.x, f.y, f.width, f.height));
        }
        fn zoom(&self) {
            self.calls.borrow_mut().push("zoom".into());
        }
        fn enter_full_screen(&self) {
            self.calls.borrow_mut().push("full screen".into());
        }
    }

    fn saved(frame: Rect) -> Saved {
        Saved { frame, maximized: false, full_screen: false }
    }

    fn calls(window: &Fake) -> Vec<String> {
        window.calls.borrow().clone()
    }

    #[test]
    fn with_nothing_saved_the_window_stays_as_it_opened() {
        let window = Fake { screens: vec![LAPTOP], ..Fake::default() };
        restore(&window, None);
        assert!(calls(&window).is_empty());
    }

    #[test]
    fn the_window_comes_back_where_it_was_fitted_to_the_screens_there_are() {
        let window = Fake { screens: vec![LAPTOP, BIG], ..Fake::default() };
        restore(&window, Some(saved(rect(-2400.0, -200.0, 1800.0, 1200.0))));
        assert_eq!(calls(&window), ["place -2400 -200 1800 1200"]);
        // The big display unplugged: on the laptop, at its size.
        let window = Fake { screens: vec![LAPTOP], ..Fake::default() };
        restore(&window, Some(saved(rect(-2400.0, -200.0, 1800.0, 1200.0))));
        assert_eq!(calls(&window), ["place 0 33 1512 870"]);
    }

    #[test]
    fn a_maximized_window_comes_back_maximized_on_the_screen_it_opens_on() {
        // Maximized on the big display, which is there: placed back on it, then maximized.
        let window = Fake { screens: vec![LAPTOP, BIG], ..Fake::default() };
        let frame = rect(-2400.0, -200.0, 1800.0, 1200.0);
        restore(&window, Some(Saved { maximized: true, ..saved(frame) }));
        assert_eq!(calls(&window), ["place -2400 -200 1800 1200", "zoom"]);
        // Opened on the laptop: placed on the laptop first, so it maximizes there. Fitted, it would
        // fill the laptop, which AppKit takes as maximized already, and maximizing would put it
        // back instead: so it goes a little in from each edge.
        let window = Fake { screens: vec![LAPTOP], ..Fake::default() };
        restore(&window, Some(Saved { maximized: true, ..saved(frame) }));
        assert_eq!(calls(&window), ["place 10 43 1492 850", "zoom"]);
    }

    #[test]
    fn a_full_screen_window_comes_back_full_screen() {
        let window = Fake { screens: vec![LAPTOP], ..Fake::default() };
        let frame = rect(100.0, 80.0, 1200.0, 700.0);
        restore(&window, Some(Saved { maximized: true, full_screen: true, ..saved(frame) }));
        assert_eq!(calls(&window), ["place 100 80 1200 700", "full screen"]);
    }

    #[test]
    fn noting_a_window_saves_its_frame() {
        let frame = rect(100.0, 80.0, 1200.0, 700.0);
        let window = Fake { frame: Some(frame), ..Fake::default() };
        assert_eq!(note(&window, None), Some(saved(frame)));
        assert_eq!(note(&window, Some(saved(LAPTOP))), Some(saved(frame)));
    }

    #[test]
    fn a_maximized_or_full_screen_window_keeps_the_frame_it_had_before() {
        let before = saved(rect(100.0, 80.0, 1200.0, 700.0));
        let zoomed = Fake { frame: Some(LAPTOP), zoomed: true, ..Fake::default() };
        assert_eq!(note(&zoomed, Some(before)), Some(Saved { maximized: true, ..before }));
        let full = Fake { frame: Some(LAPTOP), full_screen: true, ..Fake::default() };
        assert_eq!(note(&full, Some(before)), Some(Saved { full_screen: true, ..before }));
        // Maximized before anything was saved: its own frame is all there is, fitted when it
        // comes back.
        assert_eq!(note(&zoomed, None), Some(Saved { maximized: true, ..saved(LAPTOP) }));
    }

    #[test]
    fn a_window_that_cant_tell_its_frame_keeps_what_was_saved() {
        let window = Fake::default();
        assert_eq!(note(&window, None), None);
        assert_eq!(note(&window, Some(saved(LAPTOP))), Some(saved(LAPTOP)));
    }

    #[test]
    fn a_saved_frame_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new folder/window.json");
        let frame = Saved { maximized: true, ..saved(rect(-2400.5, -200.0, 1800.0, 1200.0)) };
        save(&file, &frame);
        assert_eq!(load(&file), Some(frame));
    }

    #[test]
    fn a_missing_or_broken_file_means_nothing_saved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("window.json");
        assert_eq!(load(&file), None);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(load(&file), None);
        // An older file without the flags reads as neither maximized nor full screen.
        std::fs::write(&file, r#"{"frame":{"x":1,"y":2,"width":1000,"height":700}}"#).unwrap();
        assert_eq!(load(&file), Some(saved(rect(1.0, 2.0, 1000.0, 700.0))));
    }

    #[test]
    fn saving_where_it_cant_write_does_no_harm() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be.
        save(dir.path(), &saved(LAPTOP));
        assert_eq!(load(dir.path()), None);
        // No folder at all: the root, which can't be written.
        save(Path::new("/"), &saved(LAPTOP));
    }

    #[test]
    fn frames_write_each_change_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("window.json");
        let frames = Frames::open(Some(file.clone()));
        assert_eq!(frames.saved(), None);
        let window = Fake { frame: Some(LAPTOP), ..Fake::default() };
        frames.note(Some(&window));
        assert_eq!(load(&file), Some(saved(LAPTOP)));
        // The same frame again isn't written: the file stays as it is.
        std::fs::remove_file(&file).unwrap();
        frames.note(Some(&window));
        assert_eq!(load(&file), None);
        // Writing (as on quitting) puts it back, and the next launch reads it.
        frames.write();
        assert_eq!(Frames::open(Some(file)).saved(), Some(saved(LAPTOP)));
    }

    #[test]
    fn without_a_file_frames_are_kept_but_not_written() {
        let frames = Frames::open(None);
        frames.note(Some(&Fake { frame: Some(LAPTOP), ..Fake::default() }));
        frames.note(None::<&Fake>);
        assert_eq!(frames.saved(), Some(saved(LAPTOP)));
        frames.write();
    }

    #[test]
    fn a_screens_usable_area_is_in_points() {
        let area = PhysicalRect {
            position: tauri::PhysicalPosition::new(0, 66),
            size: tauri::PhysicalSize::new(3024, 1740),
        };
        assert_eq!(points(&area, 2.0), LAPTOP);
    }

    /// Runs Moss with the plugin saving to `file`, opens the main window and clicks its red
    /// button, then deletes the file and quits.
    fn run_with(file: Option<PathBuf>) {
        let deleted = file.clone();
        let app = mock_builder()
            .plugin(plugin(move |_| file))
            .plugin(crate::window::plugin(true))
            .build(mock_context(noop_assets()))
            .unwrap();
        // A window that isn't the main one is left alone.
        tauri::WebviewWindowBuilder::new(&app, "other", Default::default()).build().unwrap();
        tauri::WebviewWindowBuilder::new(&app, MAIN, Default::default()).build().unwrap();
        app.run(move |app, event| match event {
            RunEvent::Ready => {
                for label in ["other", MAIN] {
                    app.get_webview_window(label).unwrap().close().unwrap();
                }
            }
            // The closes are handled: delete the file, and quit by leaving no windows.
            RunEvent::MainEventsCleared => {
                if let Some(file) = deleted.as_deref() {
                    let _ = std::fs::remove_file(file);
                }
                for window in app.webview_windows().into_values() {
                    window.destroy().unwrap();
                }
            }
            _ => {}
        });
    }

    #[test]
    fn moss_saves_the_window_when_it_hides_and_when_it_quits() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("window.json");
        // The test runtime's window is always at 0, 0 and 0 by 0, on no screen. Hiding saves
        // that, and quitting writes it again after the file is gone.
        save(&file, &Saved { maximized: true, full_screen: true, ..saved(LAPTOP) });
        run_with(Some(file.clone()));
        assert_eq!(load(&file), Some(saved(rect(0.0, 0.0, 0.0, 0.0))));
        // With nowhere to save, Moss still runs.
        run_with(None);
    }

    #[test]
    fn a_real_window_can_be_placed_maximized_and_made_full_screen() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, MAIN, Default::default()).build().unwrap();
        let window = window.as_ref().window();
        // The test runtime has no screens and never moves its window, so this checks only that
        // each goes through; the Fake tests check which is called when.
        assert_eq!(window.screens(), []);
        restore(&window, Some(Saved { maximized: true, ..saved(LAPTOP) }));
        assert!(!window.zoomed() && !window.in_full_screen());
        assert_eq!(window.frame(), Some(rect(0.0, 0.0, 0.0, 0.0)));
    }

    #[test]
    fn moving_resizing_and_hiding_save_the_frame() {
        assert!(saves(&WindowEvent::Moved(tauri::PhysicalPosition::new(10, 20))));
        assert!(saves(&WindowEvent::Resized(tauri::PhysicalSize::new(1000, 700))));
        assert!(!saves(&WindowEvent::Focused(true)));
    }

    fn monitor(x: i32, y: i32, area: (i32, i32, u32, u32), scale: f64) -> Monitor {
        tauri_runtime::monitor::Monitor {
            name: None,
            size: tauri::PhysicalSize::new(area.2, area.3),
            position: tauri::PhysicalPosition::new(x, y),
            work_area: PhysicalRect {
                position: tauri::PhysicalPosition::new(area.0, area.1),
                size: tauri::PhysicalSize::new(area.2, area.3),
            },
            scale_factor: scale,
        }
        .into()
    }

    #[test]
    fn screens_are_in_points_with_the_main_one_first() {
        // The LG 5K with the menu bar and Dock, and the laptop to its right, both at 2 pixels
        // to the point.
        let lg = monitor(0, 0, (0, 60, 5120, 2672), 2.0);
        let laptop = monitor(5120, 916, (5120, 980, 3024, 1900), 2.0);
        let expected = [rect(0.0, 30.0, 2560.0, 1336.0), rect(2560.0, 490.0, 1512.0, 950.0)];
        assert_eq!(screens(vec![laptop.clone(), lg.clone()], Some(lg.clone())), expected);
        // Not knowing which is main, they keep the order the system gives.
        assert_eq!(screens(vec![lg, laptop], None), expected);
    }

    #[test]
    fn the_smallest_size_is_the_one_tauri_conf_sets() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let window = &conf["app"]["windows"][0];
        assert_eq!(window["minWidth"].as_f64(), Some(MIN_SIZE.0));
        assert_eq!(window["minHeight"].as_f64(), Some(MIN_SIZE.1));
    }
}
