//! Which Moss is running: the one people install, or Moss Dev, the one developers build to try
//! changes beside it.
//!
//! - **Moss** (`app.moss.mail`) is made only by `pnpm release`. Its data folder, Keychain items,
//!   agent address and update feed are the ones every installed Moss has used since 0.1.0.
//! - **Moss Dev** (`app.moss.mail.dev`) is what `pnpm app` and `pnpm tauri dev` build, with the
//!   overlay `tauri.dev.conf.json`. Everything keyed to the app is its own: data folder, caches,
//!   Keychain items, agent port and name, Dock icon. It never checks for updates.
//!
//! The identifier in the app's config decides, and anything that isn't exactly production's is
//! Moss Dev. A debug build is always Moss Dev, so `cargo run` or a `tauri dev` without the
//! overlay can't open the installed Moss's mail either.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The installed Moss's bundle identifier.
pub const PRODUCTION: &str = "app.moss.mail";
/// Moss Dev's bundle identifier.
pub const DEVELOPMENT: &str = "app.moss.mail.dev";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Production,
    Development,
}

impl Flavor {
    /// The Moss with this bundle identifier: only production's own is production.
    pub fn of(identifier: &str) -> Self {
        if identifier == PRODUCTION { Self::Production } else { Self::Development }
    }

    pub fn identifier(self) -> &'static str {
        match self {
            Self::Production => PRODUCTION,
            Self::Development => DEVELOPMENT,
        }
    }

    /// The name in the menu bar, the Dock and the window title.
    pub fn name(self) -> &'static str {
        match self {
            Self::Production => "Moss",
            Self::Development => "Moss Dev",
        }
    }

    /// The Keychain service Moss's items are filed under: the app's own identifier. For the
    /// installed Moss that is `app.moss.mail`, the service every Moss before it used.
    pub fn keychain_service(self) -> &'static str {
        self.identifier()
    }

    /// The port the agents' (MCP) server asks for first.
    pub fn mcp_port(self) -> u16 {
        match self {
            Self::Production => moss_mcp::PORT,
            Self::Development => moss_mcp::DEV_PORT,
        }
    }

    /// The name agents know this Moss by, in the commands Settings gives to paste. Moss Dev's
    /// differs, so adding it to an agent never replaces the installed Moss.
    pub fn agent_name(self) -> &'static str {
        match self {
            Self::Production => "moss",
            Self::Development => "moss-dev",
        }
    }

    /// Whether this Moss looks for, downloads and installs new versions. Moss Dev never does: it
    /// would replace itself with the installed Moss.
    pub fn updates(self) -> bool {
        self == Self::Production
    }

    /// The file name the recovery archive's save dialog suggests.
    pub fn backup_name(self) -> String {
        format!("{} recovery.moss-backup", self.name())
    }
}

/// Makes a debug build Moss Dev whatever its config says, then says which Moss `config` is. Call
/// it on the context's config before the app is built: Tauri names the data and cache folders
/// after the identifier.
pub fn settle(config: &mut tauri::Config, debug: bool) -> Flavor {
    if debug && config.identifier == PRODUCTION {
        config.identifier = DEVELOPMENT.into();
        config.product_name = Some(Flavor::Development.name().into());
    }
    Flavor::of(&config.identifier)
}

/// Where this Moss keeps its data: its own app-data folder (`app_data`), or the folder in
/// `MOSS_DATA_DIR` (`custom`) where this Moss may use one: Moss Dev, or any debug build. The
/// installed Moss always uses its own. Moss Dev never opens the installed Moss's folder, even
/// when told to: it would update that mail for itself, and the installed Moss couldn't open it.
pub fn data_dir(flavor: Flavor, app_data: PathBuf, custom: Option<&Path>, debug: bool) -> PathBuf {
    let production = resolved(&app_data.with_file_name(PRODUCTION));
    match custom.filter(|_| debug || flavor == Flavor::Development) {
        Some(dir) if flavor == Flavor::Development && resolved(dir) == production => app_data,
        Some(dir) => dir.to_path_buf(),
        None => app_data,
    }
}

/// `path` as the file system would follow it: links resolved where they exist, `.` and `..`
/// taken out where they don't yet.
fn resolved(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            part => {
                out.push(part);
                if let Ok(real) = out.canonicalize() {
                    out = real;
                }
            }
        }
    }
    out
}

/// What the window needs to know about this Moss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIdentity {
    pub name: &'static str,
    /// Moss Dev: shows a Dev mark.
    pub dev: bool,
    /// Whether to look for updates.
    pub updates: bool,
    /// The name agents add this Moss as.
    pub agent_name: &'static str,
}

impl From<Flavor> for AppIdentity {
    fn from(flavor: Flavor) -> Self {
        Self {
            name: flavor.name(),
            dev: flavor == Flavor::Development,
            updates: flavor.updates(),
            agent_name: flavor.agent_name(),
        }
    }
}

/// Which Moss this is, for the window.
#[tauri::command]
pub fn app_identity<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> AppIdentity {
    Flavor::of(&app.config().identifier).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Flavor::*;

    #[test]
    fn only_production_s_exact_identifier_is_production() {
        assert_eq!(Flavor::of("app.moss.mail"), Production);
        for other in ["app.moss.mail.dev", "app.moss.mail.", "app.moss", "", "APP.MOSS.MAIL"] {
            assert_eq!(Flavor::of(other), Development, "{other}");
        }
        assert_eq!(Flavor::of(Production.identifier()), Production);
        assert_eq!(Flavor::of(Development.identifier()), Development);
    }

    #[test]
    fn the_installed_moss_keeps_everything_it_had() {
        assert_eq!(Production.identifier(), "app.moss.mail");
        assert_eq!(Production.name(), "Moss");
        assert_eq!(Production.keychain_service(), "app.moss.mail");
        assert_eq!(Production.keychain_service(), moss_accounts::KEYCHAIN_SERVICE);
        assert_eq!(Production.mcp_port(), 47_217);
        assert_eq!(Production.agent_name(), "moss");
        assert!(Production.updates());
        assert_eq!(Production.backup_name(), "Moss recovery.moss-backup");
    }

    #[test]
    fn moss_dev_shares_nothing_keyed_to_the_installed_moss() {
        assert_eq!(Development.identifier(), "app.moss.mail.dev");
        assert_eq!(Development.name(), "Moss Dev");
        assert_eq!(Development.keychain_service(), "app.moss.mail.dev");
        for (dev, production) in [
            (Development.identifier(), Production.identifier()),
            (Development.name(), Production.name()),
            (Development.keychain_service(), Production.keychain_service()),
            (Development.agent_name(), Production.agent_name()),
        ] {
            assert_ne!(dev, production);
        }
        assert_ne!(Development.mcp_port(), Production.mcp_port());
        assert_ne!(Development.backup_name(), Production.backup_name());
        assert!(!Development.updates());
    }

    #[test]
    fn the_shipped_config_is_production_and_the_overlay_is_moss_dev() {
        let shipped: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(shipped["identifier"], PRODUCTION);
        assert_eq!(shipped["productName"], "Moss");
        assert_eq!(shipped["app"]["windows"][0]["title"], "Moss");
        let overlay: serde_json::Value = serde_json::from_str(include_str!("../tauri.dev.conf.json")).unwrap();
        assert_eq!(overlay["identifier"], DEVELOPMENT);
        assert_eq!(overlay["productName"], "Moss Dev");
        // The overlay replaces the whole window list, so its window must match but for the title.
        let mut window = shipped["app"]["windows"][0].clone();
        window["title"] = "Moss Dev".into();
        assert_eq!(overlay["app"]["windows"], serde_json::json!([window]));
        // No update feed to read, and no update archive to make.
        assert_eq!(overlay["plugins"]["updater"]["endpoints"], serde_json::json!([]));
        assert_eq!(overlay["bundle"]["createUpdaterArtifacts"], false);
    }

    fn config(identifier: &str) -> tauri::Config {
        let mut config = tauri::Config::default();
        config.identifier = identifier.into();
        config.product_name = Some("Moss".into());
        config
    }

    #[test]
    fn a_debug_build_is_always_moss_dev() {
        let mut debug = config(PRODUCTION);
        assert_eq!(settle(&mut debug, true), Development);
        assert_eq!(debug.identifier, DEVELOPMENT);
        assert_eq!(debug.product_name.as_deref(), Some("Moss Dev"));

        let mut release = config(PRODUCTION);
        assert_eq!(settle(&mut release, false), Production);
        assert_eq!(release.identifier, PRODUCTION);
        assert_eq!(release.product_name.as_deref(), Some("Moss"));

        for debug in [true, false] {
            let mut dev = config(DEVELOPMENT);
            assert_eq!(settle(&mut dev, debug), Development);
            assert_eq!(dev.identifier, DEVELOPMENT);
        }
    }

    #[test]
    fn the_installed_moss_always_uses_its_own_data_folder() {
        let support = tempfile::tempdir().unwrap();
        let own = support.path().join(PRODUCTION);
        let custom = support.path().join("copy");
        assert_eq!(data_dir(Production, own.clone(), Some(&custom), false), own);
        assert_eq!(data_dir(Production, own.clone(), None, false), own);
    }

    #[test]
    fn moss_dev_and_debug_builds_can_keep_their_data_elsewhere() {
        let support = tempfile::tempdir().unwrap();
        let dev = support.path().join(DEVELOPMENT);
        let custom = support.path().join("copy");
        assert_eq!(data_dir(Development, dev.clone(), Some(&custom), false), custom);
        assert_eq!(data_dir(Development, dev.clone(), None, false), dev);
        let production = support.path().join(PRODUCTION);
        assert_eq!(data_dir(Production, production.clone(), Some(&custom), true), custom);
    }

    #[test]
    fn moss_dev_never_opens_the_installed_moss_s_folder() {
        let support = tempfile::tempdir().unwrap();
        let dev = support.path().join(DEVELOPMENT);
        let production = support.path().join(PRODUCTION);
        // Not there yet, written another way, there, and through a link.
        assert_eq!(data_dir(Development, dev.clone(), Some(&production), false), dev);
        assert_eq!(data_dir(Development, dev.clone(), Some(&support.path().join("x/..").join(PRODUCTION)), false), dev);
        // A folder named from here is someone else's.
        assert_eq!(data_dir(Development, dev.clone(), Some(Path::new("./moss-copy")), false), Path::new("./moss-copy"));
        std::fs::create_dir(&production).unwrap();
        assert_eq!(data_dir(Development, dev.clone(), Some(&production), true), dev);
        let link = support.path().join("link");
        std::os::unix::fs::symlink(&production, &link).unwrap();
        assert_eq!(data_dir(Development, dev.clone(), Some(&link), false), dev);
        assert_ne!(data_dir(Development, dev.clone(), None, false), production);
    }

    #[test]
    fn the_command_answers_for_the_running_app() {
        for (identifier, dev) in [(PRODUCTION, false), (DEVELOPMENT, true)] {
            let mut context = tauri::test::mock_context(tauri::test::noop_assets());
            context.config_mut().identifier = identifier.into();
            let app = tauri::test::mock_builder().build(context).unwrap();
            let identity = app_identity(app.handle().clone());
            assert_eq!((identity.dev, identity.updates), (dev, !dev), "{identifier}");
        }
    }

    #[test]
    fn the_window_learns_which_moss_it_is() {
        assert_eq!(
            AppIdentity::from(Production),
            AppIdentity { name: "Moss", dev: false, updates: true, agent_name: "moss" }
        );
        assert_eq!(
            serde_json::to_value(AppIdentity::from(Development)).unwrap(),
            serde_json::json!({"name": "Moss Dev", "dev": true, "updates": false, "agentName": "moss-dev"})
        );
    }
}
