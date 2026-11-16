//! Files that go with an email: what kind each is, and how much each mail service takes.
//! The same limits as `src/lib/attachments.ts`, which says so before you press Send.

use moss_accounts::MailService;

const MB: u64 = 1024 * 1024;

/// The most file data an email from `service` can carry. Gmail and Yahoo take 25 MB,
/// Outlook and iCloud 20 MB, Fastmail 50 MB. Resend takes 40 MB once encoded, which is
/// about 30 MB of files. Other servers say nothing ahead of time: 25 MB is the usual.
pub fn send_limit(service: MailService) -> u64 {
    MB * match service {
        MailService::Outlook | MailService::ICloud => 20,
        MailService::Fastmail => 50,
        MailService::Resend => 30,
        MailService::Gmail | MailService::Yahoo | MailService::Imap => 25,
    }
}

/// The service's name, for "Gmail sends up to 25 MB".
pub fn service_name(service: MailService) -> &'static str {
    match service {
        MailService::Gmail => "Gmail",
        MailService::Outlook => "Outlook",
        MailService::ICloud => "iCloud",
        MailService::Yahoo => "Yahoo",
        MailService::Fastmail => "Fastmail",
        MailService::Resend => "Resend",
        MailService::Imap => "Your mail server",
    }
}

/// "31.2 MB": a total as the composer shows it, to one decimal place.
pub fn megabytes(bytes: u64) -> String {
    let mb = format!("{:.1}", bytes as f64 / MB as f64);
    format!("{} MB", mb.strip_suffix(".0").unwrap_or(&mb))
}

/// Whether `total` bytes of files are too many for `service`, and if so why, in words.
pub fn too_big(total: u64, service: MailService) -> Option<String> {
    let limit = send_limit(service);
    (total > limit).then(|| {
        format!("these files are {}, and {} sends up to {}", megabytes(total), service_name(service), megabytes(limit))
    })
}

/// The content type of a file from its name, for the email to say what it is.
pub fn content_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "json" => "application/json",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pages" => "application/vnd.apple.pages",
        "numbers" => "application/vnd.apple.numbers",
        "key" => "application/vnd.apple.keynote",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Same cases as src/lib/attachments.test.ts.
    #[test]
    fn each_service_takes_its_own_amount() {
        let mb = |s| send_limit(s) / MB;
        assert_eq!(mb(MailService::Gmail), 25);
        assert_eq!(mb(MailService::Yahoo), 25);
        assert_eq!(mb(MailService::Outlook), 20);
        assert_eq!(mb(MailService::ICloud), 20);
        assert_eq!(mb(MailService::Fastmail), 50);
        assert_eq!(mb(MailService::Resend), 30);
        assert_eq!(mb(MailService::Imap), 25);
    }

    #[test]
    fn too_much_says_how_much_and_what_fits() {
        assert_eq!(too_big(25 * MB, MailService::Gmail), None);
        assert_eq!(
            too_big(25 * MB + 1, MailService::Gmail).as_deref(),
            Some("these files are 25 MB, and Gmail sends up to 25 MB")
        );
        assert_eq!(
            too_big(32_715_571, MailService::ICloud).as_deref(),
            Some("these files are 31.2 MB, and iCloud sends up to 20 MB")
        );
        assert_eq!(
            too_big(26 * MB, MailService::Imap).as_deref(),
            Some("these files are 26 MB, and Your mail server sends up to 25 MB")
        );
        for s in [MailService::Outlook, MailService::Yahoo, MailService::Fastmail, MailService::Resend] {
            assert!(too_big(51 * MB, s).unwrap().contains(service_name(s)));
        }
    }

    #[test]
    fn a_files_name_says_its_type() {
        assert_eq!(content_type("Plan.PDF"), "application/pdf");
        assert_eq!(content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(content_type("notes.txt"), "text/plain");
        assert_eq!(content_type("deck.key"), "application/vnd.apple.keynote");
        assert_eq!(content_type("README"), "application/octet-stream");
        assert_eq!(content_type("archive.tar.xyz"), "application/octet-stream");
        let all = [
            "a.log", "a.md", "a.csv", "a.html", "a.htm", "a.ics", "a.json", "a.jpg", "a.png", "a.gif", "a.webp",
            "a.heic", "a.svg", "a.mp3", "a.m4a", "a.wav", "a.mp4", "a.m4v", "a.mov", "a.zip", "a.doc", "a.docx",
            "a.xls", "a.xlsx", "a.ppt", "a.pptx", "a.pages", "a.numbers",
        ];
        for name in all {
            assert_ne!(content_type(name), "application/octet-stream", "{name}");
        }
    }
}
