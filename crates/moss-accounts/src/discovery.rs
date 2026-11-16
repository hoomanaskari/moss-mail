//! Finding IMAP and SMTP settings from just an email address.
//!
//! Same order as Thunderbird: Moss's built-in list, then the domain's own autoconfig
//! file (`autoconfig.<domain>` and `<domain>/.well-known/…`), then Mozilla's ISPDB, then
//! the domain's MX record (so a custom domain hosted by Google or Microsoft is found).
//! All autoconfig files use the Thunderbird `clientConfig` v1.1 format.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RData;
use hickory_resolver::TokioResolver;
use serde::Serialize;

use crate::connection::{domain_of, MailService, Security, Server, ServerSettings};
use crate::error::SignInError;

/// Where autoconfig files are fetched from. `{domain}` and `{email}` are filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryEndpoints {
    pub domain_autoconfig: String,
    pub well_known_autoconfig: String,
    pub ispdb: String,
}

impl Default for DiscoveryEndpoints {
    fn default() -> Self {
        Self {
            domain_autoconfig:
                "https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}".into(),
            well_known_autoconfig: "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"
                .into(),
            ispdb: "https://autoconfig.thunderbird.net/v1.1/{domain}".into(),
        }
    }
}

/// Where a set of settings came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DiscoverySource {
    /// Moss's own list of well-known services.
    Builtin,
    /// The domain's own `autoconfig.` or `.well-known` file.
    Domain,
    /// Mozilla's ISPDB.
    Ispdb,
    /// The domain's MX record points at `mail_host`.
    #[serde(rename_all = "camelCase")]
    Mx { mail_host: String },
}

/// Settings found for an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub servers: ServerSettings,
    /// The service the servers belong to. A custom domain on Google Workspace comes back
    /// as [`MailService::Gmail`], so the UI can offer the Gmail sign-in instead.
    pub service: MailService,
    pub source: DiscoverySource,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Looks up a domain's mail servers. Returns host names, most preferred first, or
/// nothing if the lookup fails.
pub trait MxLookup: Send + Sync {
    fn mail_hosts<'a>(&'a self, domain: &'a str) -> BoxFuture<'a, Vec<String>>;
}

/// MX lookups through DNS: the system's resolver, or a given server (for tests).
#[derive(Debug, Clone, Default)]
pub struct DnsMx {
    nameserver: Option<SocketAddr>,
}

impl DnsMx {
    /// Uses the system's DNS settings.
    pub fn system() -> Self {
        Self { nameserver: None }
    }

    /// Asks one DNS server over UDP.
    pub fn with_nameserver(addr: SocketAddr) -> Self {
        Self {
            nameserver: Some(addr),
        }
    }

    fn resolver(&self) -> Option<TokioResolver> {
        let mut builder = match self.nameserver {
            None => TokioResolver::builder_tokio().ok()?,
            Some(addr) => {
                let mut server = NameServerConfig::udp(addr.ip());
                for connection in &mut server.connections {
                    connection.port = addr.port();
                }
                let config = ResolverConfig::from_name_servers(vec![server]);
                TokioResolver::builder_with_config(config, TokioRuntimeProvider::default())
            }
        };
        // Sign-in is waiting on this, so give up quickly.
        builder.options_mut().timeout = std::time::Duration::from_secs(2);
        builder.options_mut().attempts = 1;
        builder.build().ok()
    }
}

impl MxLookup for DnsMx {
    fn mail_hosts<'a>(&'a self, domain: &'a str) -> BoxFuture<'a, Vec<String>> {
        // Resolver construction stays inside the future and its Tokio runtime.
        Box::pin(async move { lookup_mail_hosts(self.resolver(), domain).await })
    }
}

async fn lookup_mail_hosts(resolver: Option<TokioResolver>, domain: &str) -> Vec<String> {
    let Some(resolver) = resolver else {
        return Vec::new();
    };
    let Ok(lookup) = resolver.mx_lookup(format!("{domain}.")).await else {
        return Vec::new();
    };
    let mut records: Vec<(u16, String)> = lookup
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::MX(mx) => Some((
                mx.preference,
                mx.exchange
                    .to_ascii()
                    .trim_end_matches('.')
                    .to_ascii_lowercase(),
            )),
            _ => None,
        })
        .collect();
    records.sort();
    records
        .into_iter()
        .map(|(_, host)| host)
        .filter(|host| !host.is_empty())
        .collect()
}

/// Finds IMAP and SMTP settings for `email`, or [`SignInError::SettingsNotFound`].
pub async fn discover(
    http: &reqwest::Client,
    endpoints: &DiscoveryEndpoints,
    mx: &dyn MxLookup,
    email: &str,
) -> Result<Discovered, SignInError> {
    let domain = domain_of(email).ok_or(SignInError::InvalidEmail)?;
    let email = email.trim();
    let service = MailService::for_address(email);
    if let Some(servers) = service.known_servers(email) {
        return Ok(Discovered {
            servers,
            service,
            source: DiscoverySource::Builtin,
        });
    }

    let fill = |template: &str, domain: &str| {
        let encoded: String = url::form_urlencoded::byte_serialize(email.as_bytes()).collect();
        template
            .replace("{domain}", domain)
            .replace("{email}", &encoded)
    };
    let (own, well_known, ispdb) = tokio::join!(
        fetch_config(http, fill(&endpoints.domain_autoconfig, &domain), email),
        fetch_config(http, fill(&endpoints.well_known_autoconfig, &domain), email),
        fetch_config(http, fill(&endpoints.ispdb, &domain), email),
    );
    let found = |servers: ServerSettings, source| Discovered {
        service: service_for(&servers),
        servers,
        source,
    };
    if let Some(servers) = own.or(well_known) {
        return Ok(found(servers, DiscoverySource::Domain));
    }
    if let Some(servers) = ispdb {
        return Ok(found(servers, DiscoverySource::Ispdb));
    }

    if let Some(mail_host) = mx.mail_hosts(&domain).await.into_iter().next() {
        let source = DiscoverySource::Mx {
            mail_host: mail_host.clone(),
        };
        if let Some(service) = hosted_service(&mail_host) {
            let servers = service
                .known_servers(email)
                .expect("hosted services have known servers");
            return Ok(Discovered {
                servers,
                service,
                source,
            });
        }
        for parent in parent_domains(&mail_host, &domain) {
            if let Some(servers) = fetch_config(http, fill(&endpoints.ispdb, &parent), email).await
            {
                return Ok(found(servers, source));
            }
        }
    }
    Err(SignInError::SettingsNotFound { domain })
}

/// Google Workspace and Microsoft 365 mail hosts, which need their own sign-in.
fn hosted_service(mail_host: &str) -> Option<MailService> {
    let under = |suffix: &str| mail_host == suffix || mail_host.ends_with(&format!(".{suffix}"));
    if under("google.com") || under("googlemail.com") {
        Some(MailService::Gmail)
    } else if under("outlook.com") {
        Some(MailService::Outlook)
    } else {
        None
    }
}

/// The mail service whose servers receive a domain's mail, from its MX host: also iCloud
/// and Fastmail, which host custom domains you send from with the same login.
pub fn hosting_service(mail_host: &str) -> Option<MailService> {
    let under = |suffix: &str| mail_host == suffix || mail_host.ends_with(&format!(".{suffix}"));
    if under("icloud.com") {
        Some(MailService::ICloud)
    } else if under("messagingengine.com") {
        Some(MailService::Fastmail)
    } else {
        hosted_service(mail_host)
    }
}

/// Domains to look up in the ISPDB for an MX host: `mx1.mail.ovh.net` gives
/// `mail.ovh.net` and `ovh.net`. Skips the address's own domain (already tried).
fn parent_domains(mail_host: &str, own_domain: &str) -> Vec<String> {
    let labels: Vec<&str> = mail_host.split('.').collect();
    let mut parents = Vec::new();
    for start in 1..labels.len().saturating_sub(1) {
        let parent = labels[start..].join(".");
        if parent != own_domain && !parents.contains(&parent) {
            parents.push(parent);
        }
    }
    parents
}

/// Which service a set of servers belongs to, from the IMAP host.
fn service_for(servers: &ServerSettings) -> MailService {
    match servers.imap.host.as_str() {
        "imap.gmail.com" => MailService::Gmail,
        "outlook.office365.com" | "imap-mail.outlook.com" => MailService::Outlook,
        "imap.mail.me.com" => MailService::ICloud,
        "imap.mail.yahoo.com" => MailService::Yahoo,
        "imap.fastmail.com" => MailService::Fastmail,
        _ => MailService::Imap,
    }
}

async fn fetch_config(http: &reqwest::Client, url: String, email: &str) -> Option<ServerSettings> {
    // Any failure just means this source has nothing; the next one is tried.
    let response = http.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    parse_autoconfig(&response.text().await.ok()?, email)
}

/// Reads a Thunderbird `clientConfig` file. Picks the first IMAP and the first SMTP
/// server that use TLS or STARTTLS; servers without encryption are skipped.
pub fn parse_autoconfig(xml: &str, email: &str) -> Option<ServerSettings> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let provider = doc
        .descendants()
        .find(|n| n.has_tag_name("emailProvider"))?;
    let pick = |tag: &str, kind: &str| {
        provider
            .children()
            .filter(|n| n.has_tag_name(tag) && n.attribute("type") == Some(kind))
            .find_map(|n| parse_server(n, email))
    };
    Some(ServerSettings {
        imap: pick("incomingServer", "imap")?,
        smtp: pick("outgoingServer", "smtp")?,
    })
}

fn parse_server(node: roxmltree::Node, email: &str) -> Option<Server> {
    let text = |tag: &str| {
        node.children()
            .find(|n| n.has_tag_name(tag))
            .and_then(|n| n.text())
            .map(|t| fill_placeholders(t.trim(), email))
    };
    let security = match text("socketType")?.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => return None,
    };
    let host = text("hostname").filter(|h| !h.is_empty())?;
    let port = text("port")?.parse().ok()?;
    let username = text("username").unwrap_or_else(|| email.to_string());
    Some(Server {
        host,
        port,
        security,
        username,
    })
}

fn fill_placeholders(value: &str, email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    value
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub const CONFIG: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <domain>example.com</domain>
    <incomingServer type="pop3">
      <hostname>pop.example.com</hostname><port>995</port><socketType>SSL</socketType>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>plain.example.com</hostname><port>143</port><socketType>plain</socketType>
    </incomingServer>
    <incomingServer type="imap">
      <hostname> imap.%EMAILDOMAIN% </hostname><port>993</port><socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname><port>587</port><socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    pub fn endpoints(server: &MockServer) -> DiscoveryEndpoints {
        DiscoveryEndpoints {
            domain_autoconfig: format!("{}/own/{{domain}}?emailaddress={{email}}", server.uri()),
            well_known_autoconfig: format!("{}/well-known/{{domain}}", server.uri()),
            ispdb: format!("{}/ispdb/{{domain}}", server.uri()),
        }
    }

    /// MX answers from a table, recording which domains were asked.
    #[derive(Default)]
    pub struct FakeMx {
        pub hosts: Vec<String>,
        pub asked: Mutex<Vec<String>>,
    }

    impl FakeMx {
        pub fn new(hosts: &[&str]) -> Self {
            Self {
                hosts: hosts.iter().map(|h| h.to_string()).collect(),
                asked: Mutex::default(),
            }
        }
    }

    impl MxLookup for FakeMx {
        fn mail_hosts<'a>(&'a self, domain: &'a str) -> BoxFuture<'a, Vec<String>> {
            self.asked.lock().unwrap().push(domain.to_string());
            Box::pin(async move { self.hosts.clone() })
        }
    }

    fn xml(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(body)
    }

    async fn run(server: &MockServer, mx: &FakeMx, email: &str) -> Result<Discovered, SignInError> {
        discover(&reqwest::Client::builder().no_proxy().build().unwrap(), &endpoints(server), mx, email).await
    }

    #[test]
    fn parses_the_first_encrypted_imap_and_smtp_servers() {
        let servers = parse_autoconfig(CONFIG, "maya@example.com").unwrap();
        assert_eq!(
            servers.imap,
            Server::new("imap.example.com", 993, Security::Tls, "maya")
        );
        assert_eq!(
            servers.smtp,
            Server::new(
                "smtp.example.com",
                587,
                Security::StartTls,
                "maya@example.com"
            )
        );
    }

    #[test]
    fn rejects_configs_without_usable_servers() {
        let only_plain = CONFIG.replace("SSL", "plain").replace("STARTTLS", "plain");
        assert_eq!(parse_autoconfig(&only_plain, "m@example.com"), None);
        let no_smtp = CONFIG.replace("outgoingServer", "otherServer");
        assert_eq!(parse_autoconfig(&no_smtp, "m@example.com"), None);
        assert_eq!(parse_autoconfig("<clientConfig/>", "m@example.com"), None);
        assert_eq!(parse_autoconfig("not xml", "m@example.com"), None);
        let bad_port = CONFIG.replace("<port>993</port>", "<port>lots</port>");
        assert_eq!(parse_autoconfig(&bad_port, "m@example.com"), None);
        let no_host = CONFIG.replace(" imap.%EMAILDOMAIN% ", "");
        assert_eq!(parse_autoconfig(&no_host, "m@example.com"), None);
    }

    #[test]
    fn username_defaults_to_the_address_and_tls_is_accepted() {
        let config = CONFIG
            .replace("<username>%EMAILLOCALPART%</username>", "")
            .replace("SSL", "TLS");
        let servers = parse_autoconfig(&config, "m@example.com").unwrap();
        assert_eq!(servers.imap.username, "m@example.com");
        assert_eq!(servers.imap.security, Security::Tls);
        assert_eq!(
            fill_placeholders("%EMAILLOCALPART%", "no-at-sign"),
            "no-at-sign"
        );
    }

    #[tokio::test]
    async fn known_services_need_no_lookup() {
        let server = MockServer::start().await;
        let mx = FakeMx::default();
        let found = run(&server, &mx, " maya@icloud.com ").await.unwrap();
        assert_eq!(found.service, MailService::ICloud);
        assert_eq!(found.source, DiscoverySource::Builtin);
        assert_eq!(found.servers.imap.username, "maya@icloud.com");
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            run(&server, &mx, "nope").await,
            Err(SignInError::InvalidEmail)
        );
    }

    #[tokio::test]
    async fn the_domains_own_config_wins_over_the_ispdb() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/own/example.com"))
            .and(query_param("emailaddress", "maya@example.com"))
            .respond_with(xml(CONFIG))
            .mount(&server)
            .await;
        let ispdb = CONFIG.replace("smtp.example.com", "smtp.ispdb.example");
        Mock::given(path("/ispdb/example.com"))
            .respond_with(xml(&ispdb))
            .mount(&server)
            .await;
        let found = run(&server, &FakeMx::default(), "maya@example.com")
            .await
            .unwrap();
        assert_eq!(found.source, DiscoverySource::Domain);
        assert_eq!(found.servers.smtp.host, "smtp.example.com");
        assert_eq!(found.service, MailService::Imap);
    }

    #[tokio::test]
    async fn the_well_known_file_counts_as_the_domains_own() {
        let server = MockServer::start().await;
        Mock::given(path("/own/example.com"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/well-known/example.com"))
            .respond_with(xml(CONFIG))
            .mount(&server)
            .await;
        let found = run(&server, &FakeMx::default(), "maya@example.com")
            .await
            .unwrap();
        assert_eq!(found.source, DiscoverySource::Domain);
    }

    #[tokio::test]
    async fn falls_back_to_the_ispdb() {
        let server = MockServer::start().await;
        Mock::given(path("/own/example.com"))
            .respond_with(xml("<html>not a config</html>"))
            .mount(&server)
            .await;
        let gmail = CONFIG.replace(" imap.%EMAILDOMAIN% ", "imap.gmail.com");
        Mock::given(path("/ispdb/example.com"))
            .respond_with(xml(&gmail))
            .mount(&server)
            .await;
        let mx = FakeMx::new(&["mx.example.com"]);
        let found = run(&server, &mx, "maya@example.com").await.unwrap();
        assert_eq!(found.source, DiscoverySource::Ispdb);
        assert_eq!(found.service, MailService::Gmail);
        assert!(mx.asked.lock().unwrap().is_empty(), "MX not needed");
    }

    #[tokio::test]
    async fn google_and_microsoft_hosted_domains_are_found_by_mx() {
        let server = MockServer::start().await;
        let cases = [
            ("aspmx.l.google.com", MailService::Gmail, "imap.gmail.com"),
            (
                "northwind-co.mail.protection.outlook.com",
                MailService::Outlook,
                "outlook.office365.com",
            ),
        ];
        for (mail_host, service, imap) in cases {
            let mx = FakeMx::new(&[mail_host, "backup.example.net"]);
            let found = run(&server, &mx, "maya@northwind.co").await.unwrap();
            assert_eq!(found.service, service);
            assert_eq!(found.servers.imap.host, imap);
            assert_eq!(
                found.source,
                DiscoverySource::Mx {
                    mail_host: mail_host.into()
                }
            );
            assert_eq!(*mx.asked.lock().unwrap(), vec!["northwind.co"]);
        }
    }

    #[tokio::test]
    async fn other_mx_hosts_are_looked_up_in_the_ispdb() {
        let server = MockServer::start().await;
        Mock::given(path("/ispdb/ovh.net"))
            .respond_with(xml(CONFIG))
            .expect(1)
            .mount(&server)
            .await;
        let mx = FakeMx::new(&["mx1.mail.ovh.net"]);
        let found = run(&server, &mx, "maya@northwind.co").await.unwrap();
        assert_eq!(
            found.source,
            DiscoverySource::Mx {
                mail_host: "mx1.mail.ovh.net".into()
            }
        );
        let asked: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.url.path().to_string())
            .collect();
        assert!(asked.contains(&"/ispdb/mail.ovh.net".to_string()));
    }

    #[tokio::test]
    async fn nothing_found_is_a_typed_error() {
        let server = MockServer::start().await;
        let mx = FakeMx::new(&["mx.northwind.co"]);
        assert_eq!(
            run(&server, &mx, "maya@northwind.co").await,
            Err(SignInError::SettingsNotFound {
                domain: "northwind.co".into()
            })
        );
        assert_eq!(
            run(&server, &FakeMx::default(), "maya@northwind.co").await,
            Err(SignInError::SettingsNotFound {
                domain: "northwind.co".into()
            })
        );
    }

    #[tokio::test]
    async fn unreachable_sources_are_skipped() {
        let endpoints = DiscoveryEndpoints {
            domain_autoconfig: "http://127.0.0.1:1/{domain}".into(),
            well_known_autoconfig: "http://127.0.0.1:1/{domain}".into(),
            ispdb: "http://127.0.0.1:1/{domain}".into(),
        };
        let err = discover(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            &endpoints,
            &FakeMx::default(),
            "m@northwind.co",
        )
        .await;
        assert_eq!(
            err,
            Err(SignInError::SettingsNotFound {
                domain: "northwind.co".into()
            })
        );
    }

    #[test]
    fn parent_domains_skip_the_own_domain_and_bare_tlds() {
        assert_eq!(
            parent_domains("mx1.mail.ovh.net", "x.io"),
            vec!["mail.ovh.net", "ovh.net"]
        );
        assert_eq!(
            parent_domains("mx.northwind.co", "northwind.co"),
            Vec::<String>::new()
        );
        assert_eq!(parent_domains("localhost", "x.io"), Vec::<String>::new());
    }

    #[test]
    fn hosted_services_match_whole_labels() {
        assert_eq!(hosted_service("google.com"), Some(MailService::Gmail));
        assert_eq!(
            hosted_service("alt1.gmail-smtp-in.l.google.com"),
            Some(MailService::Gmail)
        );
        assert_eq!(hosted_service("x.googlemail.com"), Some(MailService::Gmail));
        assert_eq!(hosted_service("evilgoogle.com"), None);
        assert_eq!(hosted_service("mx.example.com"), None);
    }

    #[test]
    fn custom_domains_are_hosted_by_the_service_their_mail_goes_to() {
        assert_eq!(hosting_service("mx01.mail.icloud.com"), Some(MailService::ICloud));
        assert_eq!(hosting_service("mx02.mail.icloud.com"), Some(MailService::ICloud));
        assert_eq!(hosting_service("in1-smtp.messagingengine.com"), Some(MailService::Fastmail));
        assert_eq!(hosting_service("aspmx.l.google.com"), Some(MailService::Gmail));
        assert_eq!(hosting_service("x.mail.protection.outlook.com"), Some(MailService::Outlook));
        assert_eq!(hosting_service("evilicloud.com"), None);
        assert_eq!(hosting_service("mx.example.com"), None);
    }

    #[test]
    fn services_are_named_by_imap_host() {
        let with = |host: &str| ServerSettings {
            imap: Server::new(host, 993, Security::Tls, "u"),
            smtp: Server::new("smtp", 465, Security::Tls, "u"),
        };
        assert_eq!(
            service_for(&with("imap-mail.outlook.com")),
            MailService::Outlook
        );
        assert_eq!(service_for(&with("imap.mail.me.com")), MailService::ICloud);
        assert_eq!(
            service_for(&with("imap.mail.yahoo.com")),
            MailService::Yahoo
        );
        assert_eq!(
            service_for(&with("imap.fastmail.com")),
            MailService::Fastmail
        );
        assert_eq!(service_for(&with("imap.example.com")), MailService::Imap);
    }

    #[test]
    fn sources_serialise_for_the_ui() {
        let json = serde_json::to_value(DiscoverySource::Mx {
            mail_host: "mx.x".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "mx", "mailHost": "mx.x" })
        );
    }

    /// A one-shot DNS server that answers every MX query with `answers`.
    async fn fake_dns(answers: Vec<(u16, &'static str)>, extra_a_record: bool) -> SocketAddr {
        use hickory_resolver::proto::op::Message;
        use hickory_resolver::proto::rr::rdata::{A, MX};
        use hickory_resolver::proto::rr::{Name, Record};
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                let (n, peer) = socket.recv_from(&mut buf).await.unwrap();
                let query = Message::from_vec(&buf[..n]).unwrap();
                let mut reply = Message::response(query.metadata.id, query.metadata.op_code);
                reply.metadata.recursion_desired = query.metadata.recursion_desired;
                reply.metadata.recursion_available = true;
                let name = query.queries[0].name().clone();
                reply.add_query(query.queries[0].clone());
                for (preference, host) in &answers {
                    let mx = MX::new(*preference, Name::from_ascii(host).unwrap());
                    reply.add_answer(Record::from_rdata(name.clone(), 300, RData::MX(mx)));
                }
                if extra_a_record {
                    reply.add_answer(Record::from_rdata(
                        name.clone(),
                        300,
                        RData::A(A::new(127, 0, 0, 1)),
                    ));
                }
                socket
                    .send_to(&reply.to_vec().unwrap(), peer)
                    .await
                    .unwrap();
            }
        });
        addr
    }

    #[tokio::test]
    async fn dns_mx_returns_hosts_by_preference() {
        let addr = fake_dns(
            vec![
                (20, "backup.example.net."),
                (10, "MX1.Example.net."),
                (30, "."),
            ],
            true,
        )
        .await;
        let hosts = DnsMx::with_nameserver(addr)
            .mail_hosts("northwind.co")
            .await;
        assert_eq!(hosts, vec!["mx1.example.net", "backup.example.net"]);
    }

    #[tokio::test]
    async fn unavailable_system_dns_resolver_returns_no_guessed_mail_hosts() {
        assert!(lookup_mail_hosts(None, "example.test").await.is_empty());
    }

    #[tokio::test]
    async fn dns_mx_failures_return_nothing() {
        // Nobody answers on this port, so the lookup errors out.
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = silent.local_addr().unwrap();
        drop(silent);
        let lookup = DnsMx::with_nameserver(addr);
        let hosts = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            lookup.mail_hosts("northwind.co"),
        )
        .await;
        assert_eq!(hosts.unwrap(), Vec::<String>::new());
    }

    #[test]
    fn the_system_resolver_can_be_built() {
        // Reads the system DNS settings without sending a query.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        assert!(DnsMx::system().resolver().is_some());
        assert!(DnsMx::default().nameserver.is_none());
    }
}
