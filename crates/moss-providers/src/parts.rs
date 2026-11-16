//! Reading one part (an attachment) out of a message, by its IMAP body section.

use mail_parser::MessageParser;

/// The part numbers of `section` ("1.3" → [1, 3]) when it is one, so it's safe to put in a
/// command.
pub(crate) fn valid_section(section: &str) -> Option<Vec<u32>> {
    section
        .split('.')
        .map(|n| {
            let ok = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
            ok.then(|| n.parse().ok()).flatten().filter(|n: &u32| *n > 0)
        })
        .collect()
}

/// The decoded bytes of the part at `section` of a whole message. A message that is a
/// single part is section "1".
pub(crate) fn part(raw: &[u8], section: &str) -> Option<Vec<u8>> {
    let numbers = valid_section(section)?;
    let message = MessageParser::default().parse(raw)?;
    let mut part = message.root().root_part();
    if !part.is_multipart() {
        return (numbers == [1]).then(|| part.decoded().into_owned());
    }
    for n in numbers {
        part = part.children().nth(n as usize - 1)?;
    }
    Some(part.decoded().into_owned())
}

/// Decodes a part's body (base64, quoted-printable…) given its MIME headers, as IMAP
/// returns them for `BODY[2.MIME]` and `BODY[2]`.
pub(crate) fn decode(mime_headers: &[u8], body: &[u8]) -> Vec<u8> {
    let mut whole = header_block(mime_headers);
    whole.extend_from_slice(body);
    MessageParser::default()
        .parse(&whole[..])
        .map(|m| m.root().root_part().decoded().into_owned())
        // Only for 4 GiB or more: with the blank line there, the parser always finds a part.
        .unwrap_or_else(|| body.to_vec())
}

/// `header` ending in exactly the blank line that separates it from a body. Some servers
/// (GreenMail) leave that line out of `BODY[HEADER]` and `BODY[n.MIME]`.
pub(crate) fn header_block(header: &[u8]) -> Vec<u8> {
    let end = header.iter().rposition(|b| !matches!(b, b'\r' | b'\n')).map_or(0, |i| i + 1);
    let mut out = header[..end].to_vec();
    out.extend_from_slice(if end == 0 { b"\r\n" } else { b"\r\n\r\n" });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &[u8] = b"From: a@x.co\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n\
        Content-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n\
        <img src=\"cid:logo\">\r\n--r\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\n\
        Content-Transfer-Encoding: base64\r\n\r\nUE5H\r\n--r--\r\n--m\r\nContent-Type: application/pdf\r\n\
        Content-Transfer-Encoding: quoted-printable\r\n\
        Content-Disposition: attachment; filename=a.pdf\r\n\r\nPDF=3D1\r\n--m--\r\n";

    #[test]
    fn finds_a_part_by_its_section() {
        assert_eq!(part(MIXED, "1.2").unwrap(), b"PNG");
        assert_eq!(part(MIXED, "2").unwrap(), b"PDF=1");
        assert_eq!(part(MIXED, "1.1").unwrap(), b"<img src=\"cid:logo\">");
        assert_eq!(part(MIXED, "3"), None);
        assert_eq!(part(MIXED, "2.1"), None);
        assert_eq!(part(MIXED, "x"), None);
        assert_eq!(part(b"", "1"), None);
        let single = b"From: a@x.co\r\nContent-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nUERG\r\n";
        assert_eq!(part(single, "1").unwrap(), b"PDF");
        assert_eq!(part(single, "2"), None);
    }

    #[test]
    fn decodes_a_part_from_its_mime_headers() {
        assert_eq!(decode(b"Content-Transfer-Encoding: base64\r\n\r\n", b"UE5H\r\n"), b"PNG");
        assert_eq!(decode(b"Content-Transfer-Encoding: base64", b"UE5H"), b"PNG");
        assert_eq!(decode(b"", b"plain"), b"plain");
        let qp = b"Content-Transfer-Encoding: quoted-printable";
        for ending in ["", "\r\n", "\r\n\r\n"] {
            let headers = [&qp[..], ending.as_bytes()].concat();
            assert_eq!(decode(&headers, b"a=3Db"), b"a=b", "{ending:?}");
        }
    }

    #[test]
    fn only_part_numbers_are_sections() {
        assert_eq!(valid_section("2"), Some(vec![2]));
        assert_eq!(valid_section("1.12.3"), Some(vec![1, 12, 3]));
        for bad in ["", "0", "1.", ".1", "1..2", "2]", "2 BODY[]", "HEADER", "-1", "99999999999"] {
            assert_eq!(valid_section(bad), None, "{bad}");
        }
    }
}
