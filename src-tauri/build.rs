/// Builds Apple's helper (`apple/`, in Swift) where Tauri ships it beside Moss, as the
/// `externalBin` in `tauri.macos.conf.json` names it: `binaries/moss-apple-<target>`.
fn apple_helper() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.ends_with("apple-darwin") {
        return;
    }
    let package = std::path::Path::new("../apple");
    for watched in ["Package.swift", "Sources"] {
        println!("cargo:rerun-if-changed={}", package.join(watched).display());
    }
    let arch = if target.starts_with("aarch64") { "arm64" } else { "x86_64" };
    let swift = |extra: &[&str]| {
        let out = std::process::Command::new("swift")
            .args(["build", "-c", "release", "--arch", arch, "--package-path"])
            .arg(package)
            .args(extra)
            // Cargo's compiler settings are for Rust, not for Swift.
            .env_remove("SDKROOT")
            .env_remove("MACOSX_DEPLOYMENT_TARGET")
            .output()
            .expect("Swift (Xcode) builds Apple's helper");
        assert!(out.status.success(), "building apple/ failed:\n{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    swift(&["--product", "moss-apple"]);
    let built = std::path::Path::new(&swift(&["--show-bin-path"])).join("moss-apple");
    let binaries = std::path::Path::new("binaries");
    std::fs::create_dir_all(binaries).unwrap();
    std::fs::copy(&built, binaries.join(format!("moss-apple-{target}"))).expect("copy Apple's helper");
}

fn main() {
    apple_helper();
    let mut attributes = tauri_build::Attributes::new();
    // Tauri embeds the Common Controls manifest in the app only, so on Windows the test
    // programs start without it and die with STATUS_ENTRYPOINT_NOT_FOUND. Embed it everywhere.
    let target = |key| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest = std::env::current_dir().unwrap().join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attributes).expect("tauri build");
}
