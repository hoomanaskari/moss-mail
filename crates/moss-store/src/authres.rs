//! Whether an email really comes from the domain in its From line, as the account's own
//! mail server checked it on arrival (`Authentication-Results`, RFC 8601). A From line is
//! easy to forge, so trust by address (remote images that load by themselves) needs this.

//!
//! Which headers count. Anyone can write an `Authentication-Results` header into the mail
//! they send, so only the receiving server's own count. A server adds its headers on top of
//! the message as it arrived, so its are the topmost ones, above anything the sender wrote.
//! Moss reads the `Authentication-Results` headers from the top down and stops at the first
//! one that is not the account's server's (by its authserv-id), or that repeats an id
//! already read: a sender's copy claiming the server's id sits under the real one (servers
//! also remove such copies, RFC 8601 section 5). Only the servers Moss knows the ids of are
//! believed: Gmail (`mx.google.com`), Microsoft (no id at all, or `*.outlook.com` /
//! `*.microsoft.com`), iCloud (`*.icloud.com`, one header per check) and Fastmail
//! (`*.messagingengine.com`, `*.fastmail.com`). For any other server, which may add no header
//! at all, the topmost header could be the sender's, so nothing is confirmed.
//!
//! What confirms. `dmarc=pass` for exactly the From domain; or `dkim=pass` signed by the From
//! domain or a domain above or below it (`header.d`, else `header.i`); or `spf=pass` for such a
//! `smtp.mailfrom` domain. That alignment is DMARC's relaxed one, simplified: "the same, or a
//! subdomain either way", never a bare top-level name. `dmarc=fail` outweighs the rest.
//!
//! Limits. A server that left a sender's header claiming its id in place, and added none of
//! its own for that check, would be believed. Mail forwarded on by another service is
//! confirmed only if the account's server confirms it again.

/// A server whose own `Authentication-Results` Moss can pick out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Receiver {
    Gmail,
    Microsoft,
    ICloud,
    Fastmail,
}

impl Receiver {
    /// The account's server, from `accounts.provider` and its address.
    fn of(provider: &str, address: &str) -> Option<Receiver> {
        let domain = address.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
        match provider {
            "gmail" => Some(Receiver::Gmail),
            "microsoft" => Some(Receiver::Microsoft),
            "imap" | "jmap" => match domain.as_str() {
                "icloud.com" | "me.com" | "mac.com" => Some(Receiver::ICloud),
                "fastmail.com" | "fastmail.fm" => Some(Receiver::Fastmail),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether a header with this authserv-id (lowercase) could be this server's.
    fn owns(self, id: Option<&str>) -> bool {
        let under = |parent: &str| id.is_some_and(|id| id.ends_with(&format!(".{parent}")));
        match self {
            Receiver::Gmail => id == Some("mx.google.com"),
            Receiver::Microsoft => id.is_none() || under("outlook.com") || under("microsoft.com"),
            Receiver::ICloud => under("icloud.com"),
            Receiver::Fastmail => under("messagingengine.com") || under("fastmail.com"),
        }
    }
}

/// One check in a header: `dkim=pass header.d=example.com`, names lowercase.
#[derive(Debug, PartialEq)]
struct Check {
    method: String,
    result: String,
    properties: Vec<(String, String)>,
}

impl Check {
    fn property(&self, name: &str) -> Option<&str> {
        self.properties.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// The words of a header value between its semicolons, without comments or spacing; `=` is a
/// word of its own. Quoted text stays whole, so a semicolon inside it splits nothing.
fn statements(value: &str) -> Vec<Vec<String>> {
    let mut statements = vec![Vec::new()];
    let mut word = String::new();
    let mut comment = 0usize;
    let mut quoted = false;
    let mut chars = value.chars();
    let end = |word: &mut String, statements: &mut Vec<Vec<String>>| {
        if !word.is_empty() {
            statements.last_mut().expect("never empty").push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        if quoted {
            word.push(c);
            match c {
                '\\' => word.extend(chars.next()),
                '"' => quoted = false,
                _ => {}
            }
        } else if comment > 0 {
            match c {
                '\\' => {
                    chars.next();
                }
                '(' => comment += 1,
                ')' => comment -= 1,
                _ => {}
            }
        } else {
            match c {
                '"' => {
                    quoted = true;
                    word.push(c);
                }
                '(' => {
                    end(&mut word, &mut statements);
                    comment = 1;
                }
                ';' => {
                    end(&mut word, &mut statements);
                    statements.push(Vec::new());
                }
                '=' => {
                    end(&mut word, &mut statements);
                    statements.last_mut().expect("never empty").push("=".into());
                }
                c if c.is_whitespace() => end(&mut word, &mut statements),
                c => word.push(c),
            }
        }
    }
    end(&mut word, &mut statements);
    statements
}

/// A check from its words: `method[/version] = result` then `ptype.property = value` pairs.
fn check(words: &[String]) -> Option<Check> {
    let [method, equals, result, rest @ ..] = words else { return None };
    if equals != "=" {
        return None;
    }
    let method = method.split('/').next().unwrap_or_default().to_lowercase();
    let mut properties = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match (rest.get(i + 1), rest.get(i + 2)) {
            (Some(eq), Some(value)) if eq == "=" => {
                properties.push((rest[i].to_lowercase(), value.to_lowercase()));
                i += 3;
            }
            _ => i += 1,
        }
    }
    Some(Check { method, result: result.to_lowercase(), properties })
}

/// A header's authserv-id (lowercase; none when it goes straight into checks, as Microsoft
/// writes it) and its checks.
fn read(value: &str) -> (Option<String>, Vec<Check>) {
    let mut statements = statements(value).into_iter();
    let first = statements.next().unwrap_or_default();
    let (id, first) = if first.iter().any(|w| w == "=") {
        (None, check(&first))
    } else {
        (first.first().map(|w| w.to_lowercase()), None)
    };
    (id, first.into_iter().chain(statements.filter_map(|s| check(&s))).collect())
}

/// The domain a property names (`example.com`, `@example.com`, `bounce@example.com`), lowercase
/// and without a final dot, or `None` when it isn't a plain domain name with a dot in it.
fn domain(value: &str) -> Option<String> {
    let unquoted = value.rsplit_once('"').map_or(value, |(_, rest)| rest);
    let domain = match unquoted.rsplit_once('@') {
        Some((_, domain)) => domain,
        None if unquoted.len() == value.len() => value,
        None => return None,
    };
    let domain = domain.strip_suffix('.').unwrap_or(domain).to_ascii_lowercase();
    let plain = domain.split('.').all(|label| {
        !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    (plain && domain.contains('.')).then_some(domain)
}

/// The same domain, or one a subdomain of the other.
fn aligned(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

/// Whether the server that received this message for the account says it comes from the
/// domain of `from`. `headers` are the message's `Authentication-Results` values, top first.
pub(crate) fn sender_confirmed(provider: &str, address: &str, headers: &[String], from: &str) -> bool {
    let Some(receiver) = Receiver::of(provider, address) else { return false };
    let Some(from) = from.rsplit_once('@').and_then(|(_, d)| domain(d)) else { return false };
    let mut ids = Vec::new();
    let mut checks = Vec::new();
    for header in headers {
        let (id, found) = read(header);
        if !receiver.owns(id.as_deref()) || ids.contains(&id) {
            break;
        }
        ids.push(id);
        checks.extend(found);
    }
    let lines_up = |name: &str, check: &Check| check.property(name).and_then(domain).is_some_and(|d| aligned(&d, &from));
    if checks.iter().any(|c| c.method == "dmarc" && c.result == "fail") {
        return false;
    }
    checks.iter().filter(|c| c.result == "pass").any(|c| match c.method.as_str() {
        "dmarc" => c.property("header.from").and_then(domain).as_deref() == Some(from.as_str()),
        "dkim" => lines_up(if c.property("header.d").is_some() { "header.d" } else { "header.i" }, c),
        "spf" => lines_up("smtp.mailfrom", c),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: &str = "alex@example.com";

    fn gmail(headers: &[&str], from: &str) -> bool {
        check("gmail", "maya@gmail.com", headers, from)
    }

    fn check(provider: &str, address: &str, headers: &[&str], from: &str) -> bool {
        let headers: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
        sender_confirmed(provider, address, &headers, from)
    }

    /// Gmail's header as it arrives: one value folded over lines, comments and all.
    const GMAIL_PASS: &str = " mx.google.com;\r\n       dkim=pass header.i=@example.com header.s=s1 header.b=Ab12Cd34;\r\n       \
        spf=pass (google.com: domain of bounce@mail.example.com designates 192.0.2.10 as permitted sender) smtp.mailfrom=bounce@mail.example.com;\r\n       \
        dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=example.com\r\n";
    const GMAIL_FAIL: &str = " mx.google.com;\r\n       spf=softfail (google.com: domain of transitioning alex@example.com does not designate 198.51.100.7 as permitted sender) smtp.mailfrom=alex@example.com;\r\n       \
        dmarc=fail (p=NONE sp=NONE dis=NONE) header.from=example.com\r\n";
    const GMAIL_NOTHING: &str = " mx.google.com;\r\n       spf=neutral (google.com: 198.51.100.7 is neither permitted nor denied) smtp.mailfrom=alex@example.com\r\n";

    #[test]
    fn gmail_confirms_by_dmarc_dkim_or_spf() {
        assert!(gmail(&[GMAIL_PASS], FROM));
        assert!(gmail(&[" mx.google.com; dmarc=pass (p=NONE) header.from=example.com"], FROM));
        assert!(gmail(&[" mx.google.com; dkim=pass header.i=@example.com header.s=s1"], FROM));
        assert!(gmail(&[" mx.google.com; spf=pass (google.com: ok) smtp.mailfrom=bounce@example.com"], FROM));
        assert!(!gmail(&[GMAIL_FAIL], FROM));
        assert!(!gmail(&[GMAIL_NOTHING], FROM));
    }

    #[test]
    fn icloud_says_each_result_in_a_header_of_its_own() {
        let icloud = |headers: &[&str]| check("imap", "maya@icloud.com", headers, FROM);
        let bimi = " bimi.icloud.com; bimi=skipped reason=\"insufficient dmarc\"\r\n";
        let dmarc = " dmarc.icloud.com; dmarc=pass header.from=example.com\r\n";
        let dkim = " dkim-verifier.icloud.com;\r\n\tdkim=pass (2048-bit key) header.d=example.com header.i=@example.com header.b=Xy12\r\n";
        let spf = " spf.icloud.com; spf=pass (spf.icloud.com: domain of bounce@example.com designates 192.0.2.10 as permitted sender) smtp.mailfrom=bounce@example.com\r\n";
        let spf_fail = " spf.icloud.com; spf=fail (spf.icloud.com: domain of alex@example.com does not designate 198.51.100.7 as permitted sender) smtp.mailfrom=alex@example.com\r\n";
        let dmarc_none = " dmarc.icloud.com; dmarc=none header.from=example.com\r\n";
        assert!(icloud(&[bimi, dmarc, dkim, spf]));
        assert!(icloud(&[bimi, dmarc_none, dkim, spf_fail]));
        assert!(icloud(&[bimi, dmarc_none, spf]));
        assert!(!icloud(&[bimi, dmarc_none, spf_fail]));
        for address in ["maya@me.com", "Maya@Mac.com"] {
            assert!(check("imap", address, &[dmarc], FROM));
        }
        // iCloud's ids mean nothing to a Gmail account.
        assert!(!gmail(&[dmarc], FROM));
    }

    #[test]
    fn outlook_writes_its_header_without_an_authserv_id() {
        let outlook = |headers: &[&str]| check("microsoft", "maya@outlook.com", headers, FROM);
        let pass = " spf=pass (sender IP is 192.0.2.10)\r\n smtp.mailfrom=example.com; dkim=pass (signature was verified)\r\n header.d=example.com;dmarc=pass action=none header.from=example.com;compauth=pass\r\n reason=100\r\n";
        let fail = " spf=fail (sender IP is 198.51.100.7)\r\n smtp.mailfrom=example.com; dkim=none (message not signed)\r\n header.d=none;dmarc=fail action=none header.from=example.com;compauth=fail\r\n reason=000\r\n";
        assert!(outlook(&[pass]));
        assert!(outlook(&[" eu-west.protection.outlook.com; dkim=pass header.d=example.com"]));
        assert!(!outlook(&[fail]));
        // A second header without an id is the sender's, under Microsoft's.
        assert!(!outlook(&[fail, pass]));
        assert!(!outlook(&[" mx.google.com; dmarc=pass header.from=example.com"]));
    }

    #[test]
    fn fastmail_confirms_from_its_own_servers() {
        let fastmail = |headers: &[&str]| check("jmap", "maya@fastmail.com", headers, FROM);
        assert!(fastmail(&[" mx3.messagingengine.com;\r\n    dkim=pass (2048-bit rsa key sha256) header.d=example.com header.i=@example.com header.b=Q1w2 header.a=rsa-sha256 header.s=fm1;\r\n    dmarc=pass policy.published-domain-policy=reject header.from=example.com;\r\n    spf=pass smtp.mailfrom=bounce@example.com\r\n"]));
        assert!(check("imap", "maya@fastmail.fm", &[" mx1.messagingengine.com; spf=pass smtp.mailfrom=example.com"], FROM));
        assert!(!fastmail(&[" mx.evil.example; dmarc=pass header.from=example.com"]));
    }

    #[test]
    fn a_forged_header_under_the_servers_own_counts_for_nothing() {
        // The sender wrote a header saying pass; Gmail's, added on top, says fail.
        let forged = " mx.google.com; dkim=pass header.d=example.com; dmarc=pass header.from=example.com";
        assert!(!gmail(&[GMAIL_FAIL, forged], FROM));
        assert!(!gmail(&[GMAIL_NOTHING, forged], FROM));
        // Under a header from another server, nothing further down counts either.
        let relay = " relay.example.net; dkim=pass header.d=example.com";
        assert!(!gmail(&[GMAIL_NOTHING, relay, forged], FROM));
        assert!(!check("imap", "maya@icloud.com", &[
            " dmarc.icloud.com; dmarc=none header.from=example.com",
            " spf.icloud.com; spf=fail smtp.mailfrom=alex@example.com",
            " spf.icloud.com; spf=pass smtp.mailfrom=alex@example.com",
        ], FROM));
    }

    #[test]
    fn a_forged_header_alone_counts_for_nothing() {
        for forged in [
            " mx.evil.example; dmarc=pass header.from=example.com",
            " dmarc=pass header.from=example.com",
            " example.com; dkim=pass header.d=example.com",
        ] {
            assert!(!gmail(&[forged], FROM), "{forged}");
            assert!(!gmail(&[forged, GMAIL_PASS], FROM), "{forged}");
        }
    }

    #[test]
    fn no_header_or_a_server_that_adds_none_is_not_confirmed() {
        assert!(!gmail(&[], FROM));
        // Moss can't tell which header a server like this added, so none count.
        for (provider, address) in [("imap", "maya@northwind.co"), ("jmap", "maya@northwind.co"), ("resend", "maya@northwind.co"), ("imap", "maya@yahoo.com")] {
            assert!(!check(provider, address, &[" mx.google.com; dmarc=pass header.from=example.com"], FROM), "{provider}");
        }
    }

    #[test]
    fn names_and_results_are_read_whatever_their_case() {
        assert!(gmail(&[" MX.Google.COM; DMARC=Pass (P=REJECT) Header.From=Example.COM"], "Alex@EXAMPLE.com"));
        assert!(gmail(&[" mx.google.com; DKIM=PASS HEADER.D=EXAMPLE.COM."], FROM));
        assert!(gmail(&[" mx.google.com 1; dkim/1 = pass header.d = example.com"], FROM));
    }

    #[test]
    fn a_signature_must_be_for_the_senders_own_domain() {
        // Mail services sign as themselves too: that says nothing about the From line.
        assert!(!gmail(&[" mx.google.com; dkim=pass header.i=@sendgrid.example header.s=s1; spf=pass smtp.mailfrom=bounce@sendgrid.example; dmarc=pass header.from=sendgrid.example"], FROM));
        assert!(gmail(&[" mx.google.com; dkim=pass header.d=sendgrid.example; dkim=pass header.d=example.com"], FROM));
    }

    #[test]
    fn a_subdomain_of_the_senders_domain_lines_up_but_a_lookalike_does_not() {
        let dkim = |d: &str, from: &str| gmail(&[&format!(" mx.google.com; dkim=pass header.d={d}")], from);
        assert!(dkim("mail.example.com", FROM));
        assert!(dkim("example.com", "news@news.example.com"));
        assert!(!dkim("example.com.evil.test", FROM));
        assert!(!dkim("notexample.com", FROM));
        assert!(!dkim("example.com", "alex@example.com.evil.test"));
        assert!(!dkim("com", FROM));
        assert!(!dkim("", FROM));
        // DMARC names the From domain itself.
        assert!(!gmail(&[" mx.google.com; dmarc=pass header.from=mail.example.com"], FROM));
        assert!(!gmail(&[" mx.google.com; dmarc=pass header.from=example.com.evil.test"], FROM));
        assert!(!gmail(&[" mx.google.com; dmarc=pass"], FROM));
    }

    #[test]
    fn checks_moss_doesnt_know_confirm_nothing() {
        assert!(!gmail(&[" mx.google.com; arc=pass (i=1); bimi=pass header.d=example.com"], FROM));
    }

    #[test]
    fn dmarc_failing_outweighs_a_passing_signature() {
        assert!(!gmail(&[" mx.google.com; dkim=pass header.d=example.com; dmarc=fail header.from=example.com"], FROM));
    }

    #[test]
    fn text_inside_comments_and_quotes_is_not_read_as_results() {
        // The sender chooses the envelope address, which a server may copy into a comment.
        let sneaky = " mx.google.com; spf=fail (google.com: domain of \"x;dmarc=pass header.from=example.com\"@evil.example does not designate (nested; dkim=pass) 198.51.100.7) smtp.mailfrom=\"x;dmarc=pass header.from=example.com\"@evil.example";
        assert!(!gmail(&[sneaky], FROM));
        assert!(!gmail(&[" mx.google.com; spf=fail (relay said: ok; dkim=pass header.d=example.com ) smtp.mailfrom=evil.example"], FROM));
        assert!(!gmail(&[" mx.google.com; spf=pass smtp.mailfrom=\"alex@example.com\"@evil.example"], FROM));
        assert!(!gmail(&[" mx.google.com; spf=pass smtp.mailfrom=\"alex@example.com\""], FROM));
        assert!(!gmail(&[" mx.google.com; spf=pass smtp.mailfrom=evil.example (a \\) ; dmarc=pass header.from=example.com)"], FROM));
        assert!(gmail(&[" mx.google.com; spf=pass smtp.mailfrom=\"a \\\" b\"@example.com"], FROM));
    }

    #[test]
    fn a_from_line_without_a_proper_domain_is_never_confirmed() {
        for from in ["example.com", "alex@", "alex@exa mple.com", "alex@exämple.com"] {
            assert!(!gmail(&[GMAIL_PASS], from), "{from}");
        }
    }

    #[test]
    fn odd_headers_are_read_as_far_as_they_make_sense() {
        for odd in ["", " ;", " mx.google.com", " mx.google.com; none", " mx.google.com; dkim", " mx.google.com; dkim=", " mx.google.com; =pass", " mx.google.com; dkim=pass header.d", " mx.google.com; dkim=pass header.d=", " (unclosed", " mx.google.com; dkim pass header.d=example.com"] {
            assert!(!gmail(&[odd], FROM), "{odd:?}");
        }
    }
}
