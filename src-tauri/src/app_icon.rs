//! Moss's icon has a light and a dark look. The bundle's icon (Assets.car, from
//! `icons/Moss.icon`) switches between them as the Mac says (Light or Dark mode, or its icon
//! style). When Moss is set to Light or Dark, the UI asks for that look in the Dock instead, and
//! quitting gives the Dock back to the Mac. Which look, when, is decided in the UI
//! (`src/lib/appIcon.ts`).

use serde::Deserialize;

use crate::identity::Flavor;

/// The Dock icon: the Mac's own choice, or one look whatever the Mac is in.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppIcon {
    System,
    Light,
    Dark,
}

/// The picture for `icon` in `flavor`, drawn by `scripts/app-icon.sh --dock` from
/// `icons/Moss.icon` (Moss Dev's, with its DEV band, by `scripts/dev-icon.sh`); none for the
/// Mac's own.
pub fn png(icon: AppIcon, flavor: Flavor) -> Option<&'static [u8]> {
    match (icon, flavor) {
        (AppIcon::System, _) => None,
        (AppIcon::Light, Flavor::Production) => Some(include_bytes!("../icons/dock-light.png")),
        (AppIcon::Dark, Flavor::Production) => Some(include_bytes!("../icons/dock-dark.png")),
        (AppIcon::Light, Flavor::Development) => Some(include_bytes!("../icons/dev/dock-light.png")),
        (AppIcon::Dark, Flavor::Development) => Some(include_bytes!("../icons/dev/dock-dark.png")),
    }
}

/// Shows `icon` in the Dock while Moss runs. (Not async, so Tauri runs it on the main thread,
/// where AppKit wants it.)
#[tauri::command]
pub fn set_app_icon<R: tauri::Runtime>(app: tauri::AppHandle<R>, icon: AppIcon) {
    show(png(icon, Flavor::of(&app.config().identifier)));
}

/// Gives the Dock icon back to the Mac.
pub fn reset() {
    show(None);
}

/// The picture `png` holds, for AppKit; none if there is no picture or it can't be read.
#[cfg(target_os = "macos")]
fn image(png: Option<&[u8]>) -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    use objc2::AllocAnyThread;
    use objc2_app_kit::NSImage;
    let data = objc2_foundation::NSData::with_bytes(png?);
    NSImage::initWithData(NSImage::alloc(), &data)
}

/// Sets the Dock icon to `png`, or back to the bundle's with none. Off the main thread (as in
/// tests) there is no Dock to change.
#[cfg(target_os = "macos")]
fn show(png: Option<&'static [u8]>) {
    let image = image(png);
    let Some(main) = objc2::MainThreadMarker::new() else { return };
    // SAFETY: on the main thread; None is documented to restore the bundle's icon.
    unsafe { objc2_app_kit::NSApplication::sharedApplication(main).setApplicationIconImage(image.as_deref()) };
}

#[cfg(not(target_os = "macos"))]
fn show(_png: Option<&'static [u8]>) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Width and height from a PNG's header.
    fn size(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let at = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
        (at(16), at(20))
    }

    #[test]
    fn the_ui_names_icons_in_lowercase() {
        let icon = |s: &str| serde_json::from_value::<AppIcon>(serde_json::json!(s));
        assert_eq!(icon("system").unwrap(), AppIcon::System);
        assert_eq!(icon("light").unwrap(), AppIcon::Light);
        assert_eq!(icon("dark").unwrap(), AppIcon::Dark);
        assert!(icon("blue").is_err());
    }

    const FLAVORS: [Flavor; 2] = [Flavor::Production, Flavor::Development];

    #[test]
    fn light_and_dark_have_their_own_dock_pictures() {
        for flavor in FLAVORS {
            let light = png(AppIcon::Light, flavor).unwrap();
            let dark = png(AppIcon::Dark, flavor).unwrap();
            assert_eq!(size(light), (512, 512));
            assert_eq!(size(dark), (512, 512));
            assert_ne!(light, dark);
        }
    }

    #[test]
    fn moss_dev_never_shows_the_installed_moss_s_dock_picture() {
        for icon in [AppIcon::Light, AppIcon::Dark] {
            assert_ne!(png(icon, Flavor::Development), png(icon, Flavor::Production));
        }
    }

    #[test]
    fn the_macs_own_icon_needs_no_picture() {
        for flavor in FLAVORS {
            assert_eq!(png(AppIcon::System, flavor), None);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_reads_the_dock_pictures() {
        for (icon, flavor) in [AppIcon::Light, AppIcon::Dark].into_iter().flat_map(|i| FLAVORS.map(|f| (i, f))) {
            let size = image(png(icon, flavor)).unwrap().size();
            assert_eq!((size.width, size.height), (512.0, 512.0));
        }
        assert!(image(None).is_none());
        assert!(image(Some(b"not a picture")).is_none());
    }

    #[test]
    fn changing_the_icon_off_the_main_thread_does_nothing() {
        // Tests run off the main thread, where there is no Dock to change: nothing panics.
        let app = tauri::test::mock_app();
        set_app_icon(app.handle().clone(), AppIcon::Dark);
        reset();
    }
}
