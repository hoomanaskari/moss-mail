//! Thumbnails of attachments: a small picture of what's in a photo, PDF, video or document,
//! shown on its card in the email. Made on this Mac by Quick Look, the same thumbnails
//! Finder shows, and kept as small JPEGs in the cache folder. Only the newest few hundred
//! stay; a file that can't have one gets an empty file, so Moss doesn't fetch it again.

/// Files bigger than this aren't fetched just for a thumbnail: they show their icon.
pub(crate) const PREVIEW_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// How many thumbnails stay on disk, about 30 KB each.
pub(crate) const PREVIEWS_KEPT: usize = 500;

/// The folder for thumbnails, inside the folder of opened attachments so Settings → Storage
/// counts them and Clear cache removes them.
pub(crate) const PREVIEWS: &str = ".previews";

/// Width and height the thumbnail fits in: twice the card's 176px, for Retina screens.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SIZE: u32 = 352;

/// Extensions of files Moss draws a thumbnail of. The same lists as `canPreview` in
/// `src/lib/fileKind.ts`: pictures, PDFs, videos, documents, spreadsheets and slides.
const PREVIEWABLE: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "tif", "tiff", "bmp", "svg", // images
    "pdf", //
    "mp4", "mov", "m4v", "avi", "mkv", "webm", // video
    "doc", "docx", "pages", "odt", "rtf", // documents
    "xls", "xlsx", "numbers", "ods", "csv", "tsv", // spreadsheets
    "ppt", "pptx", "key", "odp", // slides
];

/// Whether Moss draws a thumbnail of this file: by its extension, or by its content type
/// when the name has none.
pub fn previewable(name: &str, content_type: &str) -> bool {
    let extension = name.rfind('.').filter(|&dot| dot > 0 && dot < name.len() - 1).map(|dot| &name[dot + 1..]);
    if let Some(ext) = extension {
        return PREVIEWABLE.contains(&ext.to_ascii_lowercase().as_str());
    }
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    essence == "application/pdf" || essence.starts_with("image/") || essence.starts_with("video/")
}

/// A JPEG thumbnail of a file, from Quick Look. `None` if Quick Look can't draw one in time.
#[cfg(target_os = "macos")]
pub fn thumbnail(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("moss-preview-{}-{n}", std::process::id()));
    let made = (|| {
        std::fs::create_dir_all(&dir).ok()?;
        let file = dir.join(crate::safe_file_name(name));
        std::fs::write(&file, bytes).ok()?;
        let size = SIZE.to_string();
        run(std::process::Command::new("/usr/bin/qlmanage").args(["-t", "-s", &size, "-o"]).arg(&dir).arg(&file), TIME_LIMIT)?;
        let mut png = file.into_os_string();
        png.push(".png");
        let jpeg = dir.join("thumbnail.jpg");
        let sips = ["-s", "format", "jpeg", "-s", "formatOptions", "70"];
        run(std::process::Command::new("/usr/bin/sips").args(sips).arg(png).arg("--out").arg(&jpeg), TIME_LIMIT)?;
        std::fs::read(&jpeg).ok()
    })();
    let _ = std::fs::remove_dir_all(&dir);
    made
}

/// No Quick Look elsewhere: files show their icon.
#[cfg(not(target_os = "macos"))]
pub fn thumbnail(_bytes: &[u8], _name: &str) -> Option<Vec<u8>> {
    None
}

/// How long Quick Look gets per file. It hangs on some broken files, so it's stopped then.
#[cfg(target_os = "macos")]
const TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs a command quietly; `Some` if it finished well within `limit`.
#[cfg(target_os = "macos")]
fn run(command: &mut std::process::Command, limit: std::time::Duration) -> Option<()> {
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok()? {
            return status.success().then_some(());
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same cases as `canPreview` in src/lib/fileKind.test.ts.
    #[test]
    fn photos_pdfs_videos_and_documents_get_a_thumbnail() {
        let cases = [
            ("Roof.jpg", "image/jpeg", true),
            ("Budget.pdf", "application/pdf", true),
            ("Walkthrough.mov", "video/quicktime", true),
            ("Plan.docx", "application/octet-stream", true),
            ("Pitch.key", "application/octet-stream", true),
            ("Costs.xlsx", "application/octet-stream", true),
            ("scan", "application/pdf", true),
            ("Brand assets.zip", "application/zip", false),
            ("Voice memo.m4a", "audio/mp4", false),
            ("invite.ics", "text/calendar", false),
            ("export.dat", "application/octet-stream", false),
        ];
        for (name, content_type, expected) in cases {
            assert_eq!(previewable(name, content_type), expected, "{name} ({content_type})");
        }
        assert!(previewable("IMG_0042.HEIC", ""), "any case");
        assert!(previewable("photo", "Image/PNG; name=photo"), "content type with parameters");
        assert!(!previewable(".pdf", ""), "a hidden file has no extension");
        assert!(!previewable("odd.", "application/zip"));
    }

    /// A one-page PDF with a green band across the top.
    const PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n\
        2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n\
        3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]/Contents 4 0 R/Resources<<>>>>endobj\n\
        4 0 obj<</Length 36>>stream\n0.2 0.5 0.4 rg 0 50 200 50 re f\nendstream endobj\n\
        trailer<</Root 1 0 R>>\n%%EOF\n";

    #[cfg(target_os = "macos")]
    #[test]
    fn quick_look_draws_a_small_jpeg_of_the_first_page() {
        let jpeg = thumbnail(PDF, "Budget.pdf").expect("a thumbnail");
        assert_eq!(&jpeg[..3], [0xFF, 0xD8, 0xFF], "a JPEG");
        assert!(jpeg.len() < 30_000, "small: {} bytes", jpeg.len());
        // Names from email can't put the file anywhere else.
        assert!(thumbnail(PDF, "../../Budget.pdf").is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_quick_look_cant_draw_has_none() {
        assert_eq!(thumbnail(b"not a zip", "Assets.zip"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_command_that_fails_or_hangs_gives_nothing() {
        use std::process::Command;
        use std::time::{Duration, Instant};
        assert_eq!(run(&mut Command::new("/usr/bin/true"), TIME_LIMIT), Some(()));
        assert_eq!(run(&mut Command::new("/usr/bin/false"), TIME_LIMIT), None);
        assert_eq!(run(&mut Command::new("/no/such/program"), TIME_LIMIT), None);
        let started = Instant::now();
        assert_eq!(run(Command::new("/bin/sleep").arg("60"), Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(5), "stopped, not waited for");
    }
}
