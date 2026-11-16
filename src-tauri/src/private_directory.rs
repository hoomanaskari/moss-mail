//! Restrict the dedicated desktop data directory before opening private storage.
use std::{fs, io, path::Path};

pub(crate) fn secrets(path: &Path) -> Result<Box<dyn moss_accounts::SecretStore>, Box<dyn std::error::Error>> {
    initialize(path, || Ok(moss_accounts::platform_secret_store(path)?))
}

/// No private writer runs until the directory boundary is established.
pub(crate) fn initialize<T>(path: &Path, create: impl FnOnce() -> Result<T, Box<dyn std::error::Error>>) -> Result<T, Box<dyn std::error::Error>> {
    prepare(path)?;
    create()
}

pub(crate) fn prepare(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)?;
    #[cfg(unix)]
    {
        let expected = fs::symlink_metadata(path)?;
        if !expected.is_dir() {
            return Err(io::Error::other("Moss needs a dedicated data directory, not a symbolic link."));
        }
        restrict(&fs::File::open(path)?, &expected)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict(directory: &fs::File, expected: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let actual = directory.metadata()?;
    if actual.dev() != expected.dev() || actual.ino() != expected.ino() {
        return Err(io::Error::other("The data directory changed while Moss was opening it."));
    }
    directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    verify(directory)
}

#[cfg(unix)]
fn verify(directory: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if directory.metadata()?.permissions().mode() & 0o7777 != 0o700 {
        return Err(io::Error::other("Moss could not make its data directory private."));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    // Debug builds use only an encrypted file in this disposable directory.
    #[cfg(debug_assertions)]
    #[test]
    fn development_secret_store_uses_the_private_directory_boundary() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private");
        let store = secrets(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        let reference = moss_accounts::SecretRef { service: "app.moss.fixture".into(), account: "fixture".into() };
        assert_eq!(store.load(&reference).unwrap(), None);
    }

    #[test]
    fn private_writers_run_only_after_the_directory_is_restricted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private");
        let result = initialize(&path, || {
            assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o700);
            fs::write(path.join("fixture"), b"kept")?;
            Ok(7)
        }).unwrap();
        assert_eq!(result, 7);
        assert_eq!(fs::read(path.join("fixture")).unwrap(), b"kept");
        let calls = std::cell::Cell::new(0);
        let create = || { calls.set(calls.get() + 1); Ok(()) };
        initialize(&path, create).unwrap();
        assert!(initialize(&path.join("fixture"), create).is_err());
        assert_eq!(calls.get(), 1, "a refused directory must not run a private writer");
        let error = initialize::<()>(&path, || Err(io::Error::other("fixture failure").into())).unwrap_err();
        assert_eq!(error.to_string(), "fixture failure");
    }

    #[test]
    fn verification_rejects_permissions_that_did_not_become_private() {
        let root = tempfile::tempdir().unwrap();
        let file = fs::File::open(root.path()).unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o755)).unwrap();
        assert!(verify(&file).is_err());
        file.set_permissions(fs::Permissions::from_mode(0o700)).unwrap();
        verify(&file).unwrap();
    }

    #[test]
    fn a_symlink_is_refused_without_changing_its_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(prepare(&link).is_err());
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn a_replaced_directory_is_not_chmodded() {
        let root = tempfile::tempdir().unwrap();
        let expected = fs::metadata(root.path()).unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::set_permissions(other.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(restrict(&fs::File::open(other.path()).unwrap(), &expected).is_err());
        assert_eq!(fs::metadata(other.path()).unwrap().permissions().mode() & 0o777, 0o755);
    }
}
