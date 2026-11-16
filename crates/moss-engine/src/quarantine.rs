//! Mark received files through Launch Services before exposing a path to an opener.
//! Apple documents these properties in CFURL.h and LSQuarantine.h.
use std::{io, path::Path};

pub(crate) fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    save_with(path, bytes, mark)
}

fn save_with(path: &Path, bytes: &[u8], mark: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| io::Error::other("The attachment destination is invalid."))?;
    let mut staged = tempfile::Builder::new().prefix(".moss-download-").tempfile_in(parent)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    mark(staged.path())?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use std::{ffi::c_void, os::{fd::AsRawFd, unix::ffi::OsStrExt}, ptr::{self, NonNull}};
    type Ref = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(allocator: Ref, bytes: *const u8, length: isize, directory: u8) -> Ref;
        fn CFStringCreateWithCString(allocator: Ref, text: *const std::ffi::c_char, encoding: u32) -> Ref;
        fn CFDictionaryCreate(allocator: Ref, keys: *const Ref, values: *const Ref, count: isize, key_callbacks: Ref, value_callbacks: Ref) -> Ref;
        fn CFURLSetResourcePropertyForKey(url: Ref, key: Ref, value: Ref, error: *mut Ref) -> u8;
        fn CFURLCopyResourcePropertyForKey(url: Ref, key: Ref, value: *mut Ref, error: *mut Ref) -> u8;
        fn CFURLClearResourcePropertyCache(url: Ref);
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFEqual(left: Ref, right: Ref) -> u8;
        fn CFRelease(value: Ref);
        static kCFURLQuarantinePropertiesKey: Ref;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }
    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        static kLSQuarantineTypeKey: Ref;
        static kLSQuarantineTypeEmailAttachment: Ref;
        static kLSQuarantineAgentNameKey: Ref;
    }
    struct Owned(NonNull<c_void>);
    impl Owned {
        fn new(value: Ref) -> io::Result<Self> {
            NonNull::new(value.cast_mut()).map(Self).ok_or_else(|| io::Error::other("Quarantine metadata could not be allocated."))
        }
        fn get(&self) -> Ref { self.0.as_ptr() }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            // Each value was returned by a Core Foundation Create function.
            unsafe { CFRelease(self.get()); }
        }
    }

    pub(super) fn mark(path: &Path) -> io::Result<()> {
        let path = path.canonicalize()?;
        let bytes = path.as_os_str().as_bytes();
        // Core Foundation borrows the arrays for the call. Its standard type
        // callbacks retain their entries and compare keys by CFString equality.
        let set = unsafe {
            let url = Owned::new(CFURLCreateFromFileSystemRepresentation(ptr::null(), bytes.as_ptr(), bytes.len() as isize, 0))?;
            let agent = Owned::new(CFStringCreateWithCString(ptr::null(), c"Moss".as_ptr(), 0x08000100))?;
            let keys = [kLSQuarantineTypeKey, kLSQuarantineAgentNameKey];
            let values = [kLSQuarantineTypeEmailAttachment, agent.get()];
            let properties = Owned::new(CFDictionaryCreate(ptr::null(), keys.as_ptr(), values.as_ptr(), 2, ptr::addr_of!(kCFTypeDictionaryKeyCallBacks), ptr::addr_of!(kCFTypeDictionaryValueCallBacks)))?;
            let set = CFURLSetResourcePropertyForKey(url.get(), kCFURLQuarantinePropertiesKey, properties.get(), ptr::null_mut()) != 0;
            CFURLClearResourcePropertyCache(url.get());
            let mut readback = ptr::null();
            let copied = CFURLCopyResourcePropertyForKey(url.get(), kCFURLQuarantinePropertiesKey, &mut readback, ptr::null_mut()) != 0;
            let readback = Owned::new(readback)?;
            let dictionary = CFGetTypeID(readback.get()) == CFDictionaryGetTypeID();
            let kind = if dictionary { CFDictionaryGetValue(readback.get(), kLSQuarantineTypeKey) } else { ptr::null() };
            let name = if dictionary { CFDictionaryGetValue(readback.get(), kLSQuarantineAgentNameKey) } else { ptr::null() };
            set && copied && !kind.is_null() && !name.is_null()
                && CFEqual(kind, kLSQuarantineTypeEmailAttachment) != 0 && CFEqual(name, agent.get()) != 0
        };
        let file = std::fs::File::open(path)?;
        // Unsupported resource properties may be ignored by CFURL, so success alone
        // is insufficient. Require actual quarantine metadata on the opened inode.
        let length = unsafe { libc::fgetxattr(file.as_raw_fd(), c"com.apple.quarantine".as_ptr(), ptr::null_mut(), 0, 0, 0) };
        confirmed(set, length)
    }

    fn confirmed(set: bool, length: isize) -> io::Result<()> {
        if !set || length <= 0 { return Err(io::Error::other("The attachment could not be quarantined.")); }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn missing_or_unsupported_quarantine_fails_closed() {
            for (set, bytes) in [(false, 10), (true, 0), (true, -1)] {
                assert!(confirmed(set, bytes).is_err());
            }
            assert!(confirmed(true, 10).is_ok());
            assert!(Owned::new(ptr::null()).is_err());
            let dir = tempfile::tempdir().unwrap();
            assert!(mark(&dir.path().join("missing")).is_err());
        }
    }
}

pub(crate) fn mark(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    { mac::mark(path) }
    #[cfg(not(target_os = "macos"))]
    { let _ = path; Err(io::Error::new(io::ErrorKind::Unsupported, "Attachment quarantine is not supported on this platform.")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_or_unpublishable_download_destinations_leave_existing_data_untouched() {
        let error=save_with(Path::new("/"),b"fixture",|_|panic!("invalid destination must not reach quarantine")).unwrap_err();
        assert_eq!(error.to_string(),"The attachment destination is invalid.");
        let dir=tempfile::tempdir().unwrap();let destination=dir.path().join("existing-directory");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("kept"),b"preserved").unwrap();
        assert!(save_with(&destination,b"replacement",|_|Ok(())).is_err());
        assert_eq!(std::fs::read(destination.join("kept")).unwrap(),b"preserved");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(),1,"failed publication must remove staging");
        assert!(save_with(&dir.path().join("missing/attachment"),b"fixture",|_|panic!("staging failed")).is_err());
    }

    #[test]
    fn quarantine_failure_preserves_an_existing_download_and_hides_unmarked_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attachment.pdf");
        std::fs::write(&path, b"previous good download").unwrap();
        let result = save_with(&path, b"replacement", |staged| {
            assert_ne!(staged, path.as_path(), "unmarked bytes must never be published at the open path");
            assert_eq!(std::fs::read(&path).unwrap(), b"previous good download");
            assert_eq!(std::fs::read(staged).unwrap(), b"replacement");
            Err(io::Error::other("quarantine unavailable"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous good download");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        save_with(&path, b"replacement", |_| Ok(())).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_quarantine_never_leaves_the_new_download_at_its_open_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.fileloc");
        let result = save_with(&path, b"fixture", |path| {
            assert_eq!(std::fs::read(path).unwrap(), b"fixture");
            Err(io::Error::other("fixture quarantine refusal"))
        });
        assert!(result.is_err());
        assert!(!path.exists());
        save_with(&path, b"marked fixture", |_| Ok(())).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"marked fixture");
    }
}
