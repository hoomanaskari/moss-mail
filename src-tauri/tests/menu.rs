//! The menu bar, built on the main thread as AppKit requires: so this test has its own `main`
//! (`harness = false` in Cargo.toml) instead of Rust's test threads.

use moss_lib::identity::Flavor;
use moss_lib::menu::{build, CHECK_FOR_UPDATES};
use tauri::menu::{MenuItemKind, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri_plugin_updater::UpdaterExt;

/// Each menu's title and its items' texts (`---` for a separator).
fn layout(menu: &tauri::menu::Menu<MockRuntime>) -> Vec<(String, Vec<String>)> {
    menu.items()
        .unwrap()
        .into_iter()
        .map(|item| match item {
            MenuItemKind::Submenu(sub) => {
                let texts = sub.items().unwrap().into_iter().map(|item| match item {
                    MenuItemKind::MenuItem(i) => format!("{} [{}]", i.text().unwrap(), i.id().0),
                    MenuItemKind::Predefined(p) => match p.text().unwrap() {
                        t if t.is_empty() => "---".into(),
                        t => t,
                    },
                    _ => panic!("only plain and standard items"),
                });
                (sub.text().unwrap(), texts.collect())
            }
            _ => panic!("the menu bar holds menus"),
        })
        .collect()
}

fn standard(name: &str, update: bool) -> Vec<(String, Vec<String>)> {
    let mut first = vec![format!("About {name}")];
    if update {
        first.push(format!("Check for Updates… [{CHECK_FOR_UPDATES}]"));
    }
    first.extend(["---", "Services", "---", &format!("Hide {name}"), "Hide Others", "Show All", "---", &format!("Quit {name}")].map(String::from));
    let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        (name.into(), first),
        ("File".into(), strings(&["Close Window"])),
        ("Edit".into(), strings(&["Undo", "Redo", "---", "Cut", "Copy", "Paste", "Select All"])),
        ("View".into(), strings(&["Toggle Full Screen"])),
        ("Window".into(), strings(&["Minimize", "Zoom", "---", "Close Window"])),
        ("Help".into(), vec![]),
    ]
}

/// The app as `run` builds it for `flavor`, on the test runtime.
fn app(flavor: Flavor) -> tauri::App<MockRuntime> {
    let mut context = mock_context(noop_assets());
    // The updater reads its settings when it starts, as the real config gives them.
    let shipped: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    context.config_mut().plugins.0.insert("updater".into(), shipped["plugins"]["updater"].clone());
    moss_lib::app_builder(mock_builder(), flavor).build(context).unwrap()
}

fn main() {
    // The whole app, as `run` builds it: the installed Moss has its menu and its updater.
    let production = app(Flavor::Production);
    assert_eq!(layout(&production.menu().unwrap()), standard("Moss", true));
    assert!(production.updater().is_ok());
    // Moss Dev has neither an update item nor an updater.
    let dev = app(Flavor::Development);
    assert_eq!(layout(&dev.menu().unwrap()), standard("Moss Dev", false));
    // Asking for a plugin that isn't there panics: quietly here, since that is the point.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let updater = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dev.updater().is_ok()));
    std::panic::set_hook(hook);
    assert!(!matches!(updater, Ok(true)), "Moss Dev has no updater");

    let app = mock_builder().build(mock_context(noop_assets())).unwrap();

    let production = build(app.handle(), Flavor::Production).unwrap();
    assert_eq!(layout(&production), standard("Moss", true));
    // macOS finds its Window and Help menus by these ids.
    let ids: Vec<String> = production.items().unwrap().iter().map(|m| m.id().0.clone()).collect();
    assert!(ids.contains(&WINDOW_SUBMENU_ID.to_string()) && ids.contains(&HELP_SUBMENU_ID.to_string()), "{ids:?}");

    // Moss Dev never updates, so it has no update item, and goes by its own name.
    let dev = build(app.handle(), Flavor::Development).unwrap();
    assert_eq!(layout(&dev), standard("Moss Dev", false));

    println!("menu: ok");
}
