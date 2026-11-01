//! ISPmanager 6 adapter (D-027, proposed): a read-only API client for the
//! Gate 0 capability probe. No function here changes panel state.
use crate::ispmanager_sftp::{self, SftpCheck};
use reqwest::blocking::Client;
use reqwest::{StatusCode, Url};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::time::Duration;

/// Resource categories of the D-027 coverage target, in the adapter
/// contract order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Files,
    Websites,
    Databases,
    MailDomains,
    Mailboxes,
    Cron,
    Dns,
    Ssl,
    Ftp,
    Accounts,
    Ssh,
    WebServer,
    Php,
    MailService,
    Network,
    Docker,
    PanelSettings,
    Logs,
}

impl Category {
    pub const ALL: [Category; 18] = [
        Category::Files,
        Category::Websites,
        Category::Databases,
        Category::MailDomains,
        Category::Mailboxes,
        Category::Cron,
        Category::Dns,
        Category::Ssl,
        Category::Ftp,
        Category::Accounts,
        Category::Ssh,
        Category::WebServer,
        Category::Php,
        Category::MailService,
        Category::Network,
        Category::Docker,
        Category::PanelSettings,
        Category::Logs,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Category::Files => "files",
            Category::Websites => "websites",
            Category::Databases => "databases",
            Category::MailDomains => "mail_domains",
            Category::Mailboxes => "mailboxes",
            Category::Cron => "cron",
            Category::Dns => "dns",
            Category::Ssl => "ssl",
            Category::Ftp => "ftp",
            Category::Accounts => "accounts",
            Category::Ssh => "ssh",
            Category::WebServer => "web_server",
            Category::Php => "php",
            Category::MailService => "mail_service",
            Category::Network => "network",
            Category::Docker => "docker",
            Category::PanelSettings => "panel_settings",
            Category::Logs => "logs",
        }
    }

    /// Documented list functions that the probe calls without parameters.
    /// A category without a candidate has an empty list. Form functions
    /// (`*.edit`, `license.info`, `plugin`) are absent until a live check
    /// proves that a call without `sok` changes nothing (D-027 section 3).
    pub fn list_functions(self) -> &'static [&'static str] {
        match self {
            Category::Files => &["file"],
            Category::Websites => &["webdomain", "webproxy"],
            Category::Databases => &["db", "db.server"],
            Category::MailDomains => &["emaildomain", "blacklist"],
            Category::Mailboxes => &["email", "email.sorter"],
            Category::Cron => &["scheduler"],
            Category::Dns => &["domain", "techdomain"],
            Category::Ssl => &["sslcert"],
            Category::Ftp => &["ftp.user"],
            Category::Accounts => &["user", "admin", "userstat"],
            Category::Ssh => &[],
            Category::WebServer => &["services"],
            Category::Php => &["phpversions", "phpextensions"],
            Category::MailService => &[],
            Category::Network => &["firewall", "ipdb"],
            Category::Docker => &["docker_mysql"],
            Category::PanelSettings => &["backup.storages", "backup.plan"],
            Category::Logs => &["userlogs", "authlog"],
        }
    }
}

/// The only functions that the client sends. `auth` opens a session; every
/// other entry is a documented list function.
fn is_allowed(func: &str) -> bool {
    func == "auth"
        || Category::ALL
            .iter()
            .any(|category| category.list_functions().contains(&func))
}

#[derive(Debug)]
pub enum IspError {
    Transport(reqwest::Error),
    /// The panel URL is not `https`, has no host, or carries user
    /// information or a query.
    InvalidUrl,
    /// The function is not in the read-only allowlist. Nothing was sent.
    NotAllowed(String),
    /// The panel answered with a redirect. The client follows none.
    Redirect(u16),
    Http(u16),
    /// The panel reported an error of this type (`$type` of `doc.error`).
    Panel(String),
    /// The response is not the expected `out=json` structure.
    MalformedResponse,
}

impl From<reqwest::Error> for IspError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err.without_url())
    }
}

impl fmt::Display for IspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IspError::Transport(err) => write!(f, "transport: {err}"),
            IspError::InvalidUrl => f.write_str("invalid panel URL"),
            IspError::NotAllowed(func) => write!(f, "function not allowed: {func}"),
            IspError::Redirect(code) => write!(f, "redirect refused: HTTP {code}"),
            IspError::Http(code) => write!(f, "HTTP {code}"),
            IspError::Panel(kind) => write!(f, "panel error: {kind}"),
            IspError::MalformedResponse => f.write_str("malformed response"),
        }
    }
}

pub type Result<T> = std::result::Result<T, IspError>;

/// A validated panel endpoint, for example `https://panel.example.com:1500/ispmgr`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelUrl(Url);

impl PanelUrl {
    /// A fragment, such as `#/` in the address that a browser shows, stays
    /// in the browser and never reaches the panel, so it is removed.
    pub fn parse(text: &str) -> Result<Self> {
        let mut url = Url::parse(text).map_err(|_| IspError::InvalidUrl)?;
        url.set_fragment(None);
        let valid = url.scheme() == "https"
            && url.host_str().is_some_and(|host| !host.is_empty())
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none();
        if valid {
            Ok(Self(url))
        } else {
            Err(IspError::InvalidUrl)
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn host(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }
}

/// Result of one list function, without any value from the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListOutcome {
    /// The list has elements. `fields` are the field names of the first one.
    Items {
        fields: Vec<String>,
    },
    Empty,
}

/// Reads the `doc` object of an `out=json` response, or the panel error.
fn document(body: Value) -> Result<serde_json::Map<String, Value>> {
    let Some(Value::Object(mut doc)) = body.get("doc").cloned() else {
        return Err(IspError::MalformedResponse);
    };
    if let Some(error) = doc.remove("error") {
        let kind = error
            .get("$type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(IspError::Panel(kind.to_owned()));
    }
    Ok(doc)
}

pub fn classify_list(doc: &serde_json::Map<String, Value>) -> ListOutcome {
    let first = match doc.get("elem") {
        Some(Value::Array(items)) => items.first(),
        Some(item @ Value::Object(_)) => Some(item),
        _ => None,
    };
    match first.and_then(Value::as_object) {
        Some(item) => {
            let mut fields: Vec<String> = item.keys().cloned().collect();
            fields.sort();
            ListOutcome::Items { fields }
        }
        None => ListOutcome::Empty,
    }
}

/// One authenticated panel session. The password and the session ID stay in
/// memory and travel only in a `POST` body (D-027 section 2).
pub struct Session {
    http: Client,
    url: PanelUrl,
    login: String,
    password: String,
    session_id: Option<String>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("url", &self.url)
            .field("authenticated", &self.session_id.is_some())
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn new(url: PanelUrl, login: String, password: String) -> Result<Self> {
        Ok(Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url,
            login,
            password,
            session_id: None,
        })
    }

    fn post(&self, func: &str, params: &[(&str, &str)]) -> Result<serde_json::Map<String, Value>> {
        if !is_allowed(func) {
            return Err(IspError::NotAllowed(func.to_owned()));
        }
        let mut form = vec![("func", func), ("out", "json")];
        form.extend_from_slice(params);
        let response = self.http.post(self.url.0.clone()).form(&form).send()?;
        let status = response.status();
        if status.is_redirection() {
            return Err(IspError::Redirect(status.as_u16()));
        }
        if status != StatusCode::OK {
            return Err(IspError::Http(status.as_u16()));
        }
        let body: Value = response.json().map_err(|_| IspError::MalformedResponse)?;
        document(body)
    }

    pub fn authenticate(&mut self) -> Result<()> {
        self.session_id = None;
        let doc = self.post(
            "auth",
            &[("username", &self.login), ("password", &self.password)],
        )?;
        let auth = doc.get("auth").ok_or(IspError::MalformedResponse)?;
        let id = auth
            .get("$id")
            .or_else(|| auth.get("$"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or(IspError::MalformedResponse)?;
        self.session_id = Some(id.to_owned());
        Ok(())
    }

    /// Calls a list function. An `auth` panel error means an expired
    /// session: the client authenticates again one time, then fails.
    pub fn list(&mut self, func: &str) -> Result<ListOutcome> {
        if self.session_id.is_none() {
            self.authenticate()?;
        }
        match self.list_once(func) {
            Err(IspError::Panel(kind)) if kind == "auth" => {
                self.authenticate()?;
                self.list_once(func)
            }
            other => other,
        }
    }

    fn list_once(&self, func: &str) -> Result<ListOutcome> {
        let session_id = self.session_id.as_deref().unwrap_or_default();
        let doc = self.post(func, &[("auth", session_id)])?;
        Ok(classify_list(&doc))
    }
}

/// A capability state of D-027 section 4 that a connection check can set.
/// A list read never proves a content export, so the check never sets
/// `verified_export`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    VerifiedMetadataOnly,
    PermissionDenied,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionReport {
    pub function: &'static str,
    /// `items`, `empty`, `denied` (panel error `access`), or `error`.
    pub result: &'static str,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CategoryReport {
    pub category: &'static str,
    pub state: CapabilityState,
    pub functions: Vec<FunctionReport>,
}

/// The file transport that a check confirmed (D-027 section 7a): `sftp`
/// when SFTP is ready, otherwise `panel_api` with the SFTP reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileTransport {
    pub kind: &'static str,
    /// `ready`, `host_key_unconfirmed`, `host_key_changed`,
    /// `host_certificate_unsupported`, `auth_refused`, `unreachable`, or
    /// `sftp_unavailable`.
    pub reason: &'static str,
    /// The host key that the server showed, for confirmation by the user.
    pub fingerprint: Option<String>,
    pub detail: Option<String>,
}

impl From<SftpCheck> for FileTransport {
    fn from(check: SftpCheck) -> Self {
        let api = |reason, fingerprint, detail| FileTransport {
            kind: "panel_api",
            reason,
            fingerprint,
            detail,
        };
        match check {
            SftpCheck::Ready => FileTransport {
                kind: "sftp",
                reason: "ready",
                fingerprint: None,
                detail: None,
            },
            SftpCheck::HostKeyUnconfirmed { fingerprint } => {
                api("host_key_unconfirmed", Some(fingerprint), None)
            }
            SftpCheck::HostKeyChanged { fingerprint } => {
                api("host_key_changed", Some(fingerprint), None)
            }
            SftpCheck::HostCertificateUnsupported => {
                api("host_certificate_unsupported", None, None)
            }
            SftpCheck::AuthRefused => api("auth_refused", None, None),
            SftpCheck::Unreachable(detail) => api("unreachable", None, Some(detail)),
            SftpCheck::SftpUnavailable(detail) => api("sftp_unavailable", None, Some(detail)),
        }
    }
}

/// The result of one connection check. `status` is a Registry status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub status: &'static str,
    pub error: Option<String>,
    pub categories: Vec<CategoryReport>,
    pub file_transport: Option<FileTransport>,
}

fn category_state(functions: &[FunctionReport]) -> CapabilityState {
    if functions
        .iter()
        .any(|report| matches!(report.result, "items" | "empty"))
    {
        CapabilityState::VerifiedMetadataOnly
    } else if !functions.is_empty() && functions.iter().all(|report| report.result == "denied") {
        CapabilityState::PermissionDenied
    } else {
        CapabilityState::Unverified
    }
}

/// SFTP settings of a check: the port and the confirmed host key.
#[derive(Debug, Clone, Copy)]
pub struct SftpTarget<'a> {
    pub port: u16,
    pub host_key_sha256: Option<&'a str>,
}

/// Signs in, calls the list functions of every category, and checks the
/// SFTP file transport. The check reads metadata only and changes nothing on
/// the server.
pub fn check(url: &str, login: &str, password: &str, sftp: SftpTarget<'_>) -> CheckReport {
    let failed = |status, error: IspError| CheckReport {
        status,
        error: Some(error.to_string()),
        categories: Vec::new(),
        file_transport: None,
    };
    let url = match PanelUrl::parse(url.trim()) {
        Ok(url) => url,
        Err(error) => return failed("error", error),
    };
    let host = url.host().to_owned();
    let mut session = match Session::new(url, login.to_owned(), password.to_owned()) {
        Ok(session) => session,
        Err(error) => return failed("error", error),
    };
    if let Err(error) = session.authenticate() {
        let status = match &error {
            IspError::Panel(kind) if kind == "auth" => "requires_auth",
            IspError::Transport(_) => "offline",
            _ => "error",
        };
        return failed(status, error);
    }
    let categories = Category::ALL
        .iter()
        .map(|category| {
            let functions: Vec<FunctionReport> = category
                .list_functions()
                .iter()
                .map(|function| match session.list(function) {
                    Ok(ListOutcome::Items { .. }) => FunctionReport {
                        function,
                        result: "items",
                        error: None,
                    },
                    Ok(ListOutcome::Empty) => FunctionReport {
                        function,
                        result: "empty",
                        error: None,
                    },
                    Err(error) => FunctionReport {
                        function,
                        result: match &error {
                            IspError::Panel(kind) if kind == "access" => "denied",
                            _ => "error",
                        },
                        error: Some(error.to_string()),
                    },
                })
                .collect();
            CategoryReport {
                category: category.key(),
                state: category_state(&functions),
                functions,
            }
        })
        .collect();
    let file_transport =
        ispmanager_sftp::check(&host, sftp.port, login, password, sftp.host_key_sha256);
    CheckReport {
        status: "connected",
        error: None,
        categories,
        file_transport: Some(file_transport.into()),
    }
}
