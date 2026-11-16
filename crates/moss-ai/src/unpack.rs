//! Unpacks the downloaded llama.cpp build (a .tar.gz) next to the models.

use std::path::Path;

/// Unpacks `archive` into `into`. The tar crate refuses paths that climb out with `..`.
pub fn unpack(archive: &Path, into: &Path) -> std::io::Result<()> {
    let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
    std::fs::create_dir_all(into)?;
    tar::Archive::new(gz).unpack(into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("rt.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "llama-b11960/llama-server", &b"\x7fELF"[..])
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        path
    }

    #[test]
    fn unpacks_the_runtime_and_keeps_it_runnable() {
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("runtime");
        unpack(&archive(dir.path()), &into).unwrap();
        let server = into.join("llama-b11960").join("llama-server");
        assert_eq!(std::fs::read(&server).unwrap(), b"\x7fELF");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&server).unwrap().permissions().mode() & 0o111,
                0o111
            );
        }
    }

    #[test]
    fn a_broken_archive_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.tar.gz");
        std::fs::write(&bad, b"not gzip").unwrap();
        assert!(unpack(&bad, dir.path()).is_err());
        assert!(unpack(&dir.path().join("missing.tar.gz"), dir.path()).is_err());
    }
}
