//! The menu bar: macOS's standard menus as Tauri makes them by default (About, Services, Hide,
//! Quit, Edit with Undo to Select All, View, Window, Help), plus "Check for Updates…" under
//! About in the installed Moss. Moss Dev never updates, so it has no such item. Every key Moss
//! handles itself (⌘K, ⌘J, ⌘, …) still reaches the window: the menu claims none of them.
//!
//! AppKit only makes menus on the main thread, so `build` is checked by `tests/menu.rs`, which
//! runs there.

use tauri::menu::{AboutMetadata, IsMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::identity::Flavor;

/// The menu item that checks for updates, and what the window hears when it is chosen.
pub const CHECK_FOR_UPDATES: &str = "check-for-updates";
/// The window event a chosen menu item is sent as, with the item's id.
pub const MENU_EVENT: &str = "moss://menu";

/// The items of one menu.
type Items<R> = Vec<Box<dyn IsMenuItem<R>>>;

/// The menu bar for `flavor`.
pub fn build<R: Runtime>(app: &AppHandle<R>, flavor: Flavor) -> tauri::Result<Menu<R>> {
    let name = flavor.name();
    let about = AboutMetadata { name: Some(name.into()), version: Some(app.package_info().version.to_string()), ..Default::default() };
    let line = || -> tauri::Result<Box<dyn IsMenuItem<R>>> { Ok(Box::new(PredefinedMenuItem::separator(app)?)) };
    let standard = |item: tauri::Result<PredefinedMenuItem<R>>| -> tauri::Result<Box<dyn IsMenuItem<R>>> { Ok(Box::new(item?)) };
    // Named here: AppKit would take the name from the program, which is "Moss" in Moss Dev too.
    let mut first: Items<R> = vec![standard(PredefinedMenuItem::about(app, Some(&format!("About {name}")), Some(about)))?];
    if flavor.updates() {
        first.push(Box::new(MenuItem::with_id(app, CHECK_FOR_UPDATES, "Check for Updates…", true, None::<&str>)?));
    }
    first.extend([line()?, standard(PredefinedMenuItem::services(app, None))?, line()?]);
    first.extend([standard(PredefinedMenuItem::hide(app, Some(&format!("Hide {name}"))))?, standard(PredefinedMenuItem::hide_others(app, None))?]);
    first.extend([standard(PredefinedMenuItem::show_all(app, None))?, line()?, standard(PredefinedMenuItem::quit(app, Some(&format!("Quit {name}"))))?]);
    let file: Items<R> = vec![standard(PredefinedMenuItem::close_window(app, None))?];
    let mut edit: Items<R> = vec![standard(PredefinedMenuItem::undo(app, None))?, standard(PredefinedMenuItem::redo(app, None))?, line()?];
    edit.extend([standard(PredefinedMenuItem::cut(app, None))?, standard(PredefinedMenuItem::copy(app, None))?]);
    edit.extend([standard(PredefinedMenuItem::paste(app, None))?, standard(PredefinedMenuItem::select_all(app, None))?]);
    let view: Items<R> = vec![standard(PredefinedMenuItem::fullscreen(app, None))?];
    let mut window: Items<R> = vec![standard(PredefinedMenuItem::minimize(app, None))?, standard(PredefinedMenuItem::maximize(app, None))?];
    window.extend([line()?, standard(PredefinedMenuItem::close_window(app, None))?]);
    let menus = [
        submenu(app, None, name, first)?,
        submenu(app, None, "File", file)?,
        submenu(app, None, "Edit", edit)?,
        submenu(app, None, "View", view)?,
        // macOS finds its Window and Help menus by these ids.
        submenu(app, Some(WINDOW_SUBMENU_ID), "Window", window)?,
        submenu(app, Some(HELP_SUBMENU_ID), "Help", vec![])?,
    ];
    Menu::with_items(app, &menus.iter().map(|m| m as &dyn IsMenuItem<R>).collect::<Vec<_>>())
}

/// A menu called `title` holding `items`, with `id` if macOS needs to find it.
fn submenu<R: Runtime>(app: &AppHandle<R>, id: Option<&str>, title: &str, items: Items<R>) -> tauri::Result<Submenu<R>> {
    let items: Vec<&dyn IsMenuItem<R>> = items.iter().map(|item| item.as_ref()).collect();
    match id {
        Some(id) => Submenu::with_id_and_items(app, id, title, true, &items),
        None => Submenu::with_items(app, title, true, &items),
    }
}

/// A chosen menu item: "Check for Updates…" brings the window back (the red button may have
/// hidden it) and tells it to check, so the answer is in front of you.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, id: &MenuId) {
    if id == CHECK_FOR_UPDATES {
        crate::window::reopen(app.get_webview_window("main").as_ref());
        let _ = app.emit(MENU_EVENT, CHECK_FOR_UPDATES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Listener;

    #[test]
    fn the_window_listens_for_these_names() {
        // src/services/menu.ts has the same names.
        assert_eq!(MENU_EVENT, "moss://menu");
        assert_eq!(CHECK_FOR_UPDATES, "check-for-updates");
    }

    #[test]
    fn check_for_updates_tells_the_window_and_nothing_else_does() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        // The test runtime keeps no visibility, so `window::reopen`'s own tests check that part.
        let _window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        app.listen(MENU_EVENT, move |e| sink.lock().unwrap().push(e.payload().to_string()));
        on_event(app.handle(), &MenuId::new("something-else"));
        assert!(heard.lock().unwrap().is_empty());
        on_event(app.handle(), &MenuId::new(CHECK_FOR_UPDATES));
        assert_eq!(*heard.lock().unwrap(), [format!("\"{CHECK_FOR_UPDATES}\"")]);
    }
}
