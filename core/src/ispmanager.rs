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

/// Reads of one parent object (D-027 section 7b): a nested list with `elid`
/// and an edit form opened without `sok`. Without `sok` the panel only
/// returns the form; it saves nothing.
const OBJECT_READS: [&str; 6] = [
    "db.users",
    "db.users.edit",
    "domain.record",
    "domain.record.edit",
    "email.edit",
    "sslcert.edit",
];

/// The only functions that the client sends. `auth` opens a session; every
/// other entry is a documented list function or an object read.
fn is_allowed(func: &str) -> bool {
    func == "auth"
        || OBJECT_READS.contains(&func)
        || Category::ALL
            .iter()
            .any(|category| category.list_functions().contains(&func))
}

/// The only parameters that the client sends. `sok` (save a form) is never
/// one of them.
fn is_allowed_param(name: &str) -> bool {
    matches!(name, "auth" | "username" | "password" | "elid" | "plid")
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
        if !is_allowed(func) || params.iter().any(|(name, _)| !is_allowed_param(name)) {
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
        Ok(classify_list(&self.read_once(func, &[])?))
    }

    /// Calls an allowed function with `elid` or `plid` and returns the
    /// `doc` object. It re-authenticates one time, as `list` does.
    pub fn read(
        &mut self,
        func: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Map<String, Value>> {
        if self.session_id.is_none() {
            self.authenticate()?;
        }
        match self.read_once(func, params) {
            Err(IspError::Panel(kind)) if kind == "auth" => {
                self.authenticate()?;
                self.read_once(func, params)
            }
            other => other,
        }
    }

    fn read_once(
        &self,
        func: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Map<String, Value>> {
        let session_id = self.session_id.as_deref().unwrap_or_default();
        let mut form = vec![("auth", session_id)];
        form.extend_from_slice(params);
        self.post(func, &form)
    }
}

/// The elements of a list `doc`.
fn elements(doc: &serde_json::Map<String, Value>) -> Vec<&serde_json::Map<String, Value>> {
    match doc.get("elem") {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_object).collect(),
        Some(Value::Object(item)) => vec![item],
        _ => Vec::new(),
    }
}

/// The text of a field. In `out=json` a value is a string or `{"$": ..}`.
fn text(object: &serde_json::Map<String, Value>, field: &str) -> Option<String> {
    let value = object.get(field)?;
    value
        .as_str()
        .or_else(|| value.get("$").and_then(Value::as_str))
        .map(str::to_owned)
}

fn sorted_fields(object: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut fields: Vec<String> = object.keys().cloned().collect();
    fields.sort();
    fields
}

/// The first field of `candidates` that has a non-empty value.
fn key_field(
    object: &serde_json::Map<String, Value>,
    candidates: &[&'static str],
) -> Option<(&'static str, String)> {
    candidates.iter().find_map(|field| {
        text(object, field)
            .filter(|value| !value.is_empty())
            .map(|value| (*field, value))
    })
}

/// A capability state of D-027 section 4 that a connection check can set.
/// A list read never proves a content export, so the check never sets
/// `verified_export`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    VerifiedMetadataOnly,
    PermissionDenied,
    /// The panel has none of the functions (panel error `missed`).
    Unsupported,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionReport {
    pub function: &'static str,
    /// `items`, `empty`, `denied` (panel error `access`), `missing` (panel
    /// error `missed`: the panel has no such function), or `error`.
    pub result: &'static str,
    pub error: Option<String>,
    /// The field names of the first element: the API schema, no values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

/// Live facts for the export design of mail, Cron, and DNS (D-027 section
/// 10). It holds field names and yes/no facts only: no name, no value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExportProbe {
    /// `domain.record` of the first DNS zone.
    pub dns_records: Option<FunctionReport>,
    /// The form `domain.record.edit` of the first MX record, opened without
    /// `sok`: its field names. The list has no MX priority.
    pub dns_record_form: Option<Vec<String>>,
    /// The account home has an `email` folder (SFTP).
    pub mail_folder: Option<bool>,
    /// A mailbox folder below `email/<domain>/`, or its `.maildir`, has
    /// `cur`, `new`, and `tmp`.
    pub maildir: Option<bool>,
    /// The form of the `email` list: counts and field names, no value.
    pub mailbox_names: Option<MailboxNameProbe>,
    /// The form `email.edit` of the first mailbox, opened without `sok`:
    /// its field names (forwarding, autoresponder).
    pub mailbox_form: Option<Vec<String>>,
    /// `email.sorter` with the first mailbox as `elid`.
    pub mailbox_sorter: Option<FunctionReport>,
    /// The form `sslcert.edit` of the first certificate, without `sok`.
    pub ssl_form: Option<SslFormProbe>,
    /// The CAA, TLSA, and DS records of the first DNS zone: one entry for
    /// each type that the zone has.
    pub dns_extra_records: Vec<DnsRecordProbe>,
}

/// The field names of a certificate form, and which fields hold PEM text.
/// It keeps no value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SslFormProbe {
    pub fields: Vec<String>,
    /// Fields whose value has `-----BEGIN CERTIFICATE-----`.
    pub certificate_fields: Vec<String>,
    /// Fields whose value has `PRIVATE KEY`. QADREN never exports them.
    pub private_key_fields: Vec<String>,
}

/// One record type of a DNS zone: how many space-separated parts the list
/// `value` has, and the field names of its form. It keeps no value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DnsRecordProbe {
    pub rtype: String,
    pub value_parts: usize,
    pub form_fields: Option<Vec<String>>,
}

/// How the `email` list names a mailbox and ties it to its mail domain.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MailboxNameProbe {
    pub mailboxes: usize,
    /// Mailboxes whose `name` has `@`.
    pub names_with_at: usize,
    /// Mailboxes whose `name` ends with `@<a mail domain>`.
    pub names_with_domain: usize,
    /// Fields whose value equals the name of a mail domain, in any mailbox.
    pub domain_fields: Vec<String>,
    /// Elements of `email` with `elid` = the first mail domain.
    pub with_elid: Option<usize>,
    /// Elements of `email` with `plid` = the first mail domain.
    pub with_plid: Option<usize>,
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

/// What the panel and the SSH server tell about databases (D-027 section
/// 7b). It holds field names, the server type and version fields, tool
/// versions, and whether a password is readable. It never holds a password,
/// a database name, or a user name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DatabaseProbe {
    /// Field names of the first `db` element.
    pub db_fields: Vec<String>,
    /// The `db` field used as `elid`.
    pub db_key_field: Option<&'static str>,
    /// Values of the type, version, and host fields of the first database.
    pub server_info: Vec<ServerField>,
    /// `items`, `empty`, `denied`, `missing`, or `error`.
    pub users_result: Option<&'static str>,
    pub user_fields: Vec<String>,
    pub user_key_field: Option<&'static str>,
    /// The result of the user edit form, read without `sok`.
    pub user_form_result: Option<&'static str>,
    pub user_form_fields: Vec<String>,
    /// True when the form returns a non-empty password field.
    pub password_readable: Option<bool>,
    /// True when the password field holds one repeated mask character.
    pub password_masked: Option<bool>,
    /// MySQL sign-in with the panel facts through `mysqldump --no-data`.
    pub mysql_login: Option<crate::ispmanager_sftp::MysqlLoginCheck>,
    /// `--version` of each dump tool through SSH; `None` when it is absent.
    pub tools: Vec<ToolVersion>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerField {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolVersion {
    pub tool: crate::ispmanager_sftp::DumpTool,
    pub version: Option<String>,
}

fn result_class(error: &IspError) -> &'static str {
    match error {
        IspError::Panel(kind) if kind == "access" => "denied",
        IspError::Panel(kind) if kind == "missed" => "missing",
        _ => "error",
    }
}

/// Reads the first database, its first user, and that user's edit form.
/// The access facts for a MySQL sign-in stay in memory.
fn probe_databases(session: &mut Session) -> (DatabaseProbe, Option<ispmanager_sftp::MysqlAccess>) {
    let mut probe = DatabaseProbe::default();
    let access = probe_databases_into(session, &mut probe);
    (probe, access)
}

fn probe_databases_into(
    session: &mut Session,
    probe: &mut DatabaseProbe,
) -> Option<ispmanager_sftp::MysqlAccess> {
    let doc = match session.read("db", &[]) {
        Ok(doc) => doc,
        Err(error) => {
            probe.error = Some(error.to_string());
            return None;
        }
    };
    let db = elements(&doc).first().copied().cloned()?;
    probe.db_fields = sorted_fields(&db);
    probe.server_info = db
        .keys()
        .filter(|field| {
            ["type", "version", "host", "server"]
                .iter()
                .any(|part| field.contains(part))
        })
        .filter_map(|field| {
            let value = text(&db, field)?;
            (value.len() <= 64).then(|| ServerField {
                field: field.clone(),
                value,
            })
        })
        .collect();
    let (db_field, db_key) = key_field(&db, &["pair", "key", "id", "name"])?;
    probe.db_key_field = Some(db_field);
    let users = match session.read("db.users", &[("elid", &db_key)]) {
        Ok(users) => users,
        Err(error) => {
            probe.users_result = Some(result_class(&error));
            probe.error = Some(error.to_string());
            return None;
        }
    };
    let Some(user) = elements(&users).first().copied().cloned() else {
        probe.users_result = Some("empty");
        return None;
    };
    probe.users_result = Some("items");
    probe.user_fields = sorted_fields(&user);
    let (user_field, user_key) = key_field(&user, &["key", "id", "name"])?;
    probe.user_key_field = Some(user_field);
    match session.read("db.users.edit", &[("elid", &user_key), ("plid", &db_key)]) {
        Ok(form) => {
            probe.user_form_result = Some("items");
            probe.user_form_fields = sorted_fields(&form)
                .into_iter()
                .filter(|field| !field.starts_with('$') && !field.starts_with("tparams"))
                .collect();
            let password = text(&form, "password").filter(|value| !value.is_empty());
            probe.password_readable = Some(password.is_some());
            probe.password_masked = password.as_deref().map(masked_password);
            let is_mysql = text(&db, "server")
                .is_some_and(|server| server.to_ascii_lowercase().contains("mysql"));
            let port = text(&db, "server_hostandport").and_then(|value| {
                value
                    .rsplit_once(':')
                    .and_then(|(_, port)| port.parse().ok())
            });
            match (password, text(&form, "username"), text(&db, "name")) {
                (Some(password), Some(user), Some(database))
                    if is_mysql && probe.password_masked == Some(false) =>
                {
                    return Some(ispmanager_sftp::MysqlAccess {
                        database,
                        user,
                        password,
                        host: text(&db, "server_host").unwrap_or_else(|| "localhost".to_owned()),
                        port,
                    });
                }
                _ => {}
            }
        }
        Err(error) => {
            probe.user_form_result = Some(result_class(&error));
            probe.error = Some(error.to_string());
        }
    }
    None
}

/// True when the panel shows a mask (one repeated mask character) in place
/// of the password.
fn masked_password(value: &str) -> bool {
    let mut chars = value.chars();
    let first = chars.next();
    first.is_some_and(|first| "*•●x".contains(first)) && chars.all(|ch| Some(ch) == first)
}

/// Why the access facts of one database are not available. `user` is the
/// MySQL user name when the panel gave it, so that the Scenario editor can
/// ask for the password of that user. It never goes into a log.
#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseAccessError {
    pub code: &'static str,
    pub user: Option<String>,
}

impl fmt::Debug for DatabaseAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code)
    }
}

impl From<&'static str> for DatabaseAccessError {
    fn from(code: &'static str) -> Self {
        Self { code, user: None }
    }
}

/// Where the password of a database user comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSource {
    /// The panel gave the password.
    Panel,
    /// The panel hides the password; the user entered it in the Scenario
    /// editor (owner decision 2026-11-13).
    Manual,
}

/// The MySQL access facts of one selected database, read from the panel at
/// the start of each export (D-027 section 7b). QADREN never stores them.
/// The user is the first user of the database with the read right. When the
/// panel hides the password of that user, `manual` gives the password that
/// the user entered for this MySQL user name.
pub fn mysql_access(
    session: &mut Session,
    database: &str,
    manual: &dyn Fn(&str) -> Option<String>,
) -> std::result::Result<(ispmanager_sftp::MysqlAccess, PasswordSource), DatabaseAccessError> {
    let doc = session
        .read("db", &[])
        .map_err(|_| "ispmanager_database_list_failed")?;
    let db = elements(&doc)
        .into_iter()
        .find(|db| text(db, "name").as_deref() == Some(database))
        .cloned()
        .ok_or("ispmanager_database_missing")?;
    if !text(&db, "server").is_some_and(|server| server.to_ascii_lowercase().contains("mysql")) {
        return Err("ispmanager_database_unsupported".into());
    }
    // Each step has its own code, so the run result tells which step stopped.
    let (_, db_key) = key_field(&db, &["pair", "key", "id", "name"])
        .ok_or("ispmanager_database_users_unreadable")?;
    let users = session
        .read("db.users", &[("elid", &db_key)])
        .map_err(|_| "ispmanager_database_users_unreadable")?;
    let users = elements(&users);
    let readable = |user: &&&serde_json::Map<String, Value>| {
        text(user, "read")
            .is_some_and(|value| matches!(value.as_str(), "on" | "yes" | "1" | "true"))
    };
    let user = users
        .iter()
        .find(readable)
        .or_else(|| users.first())
        .ok_or("ispmanager_database_no_users")?;
    let (_, user_key) = key_field(user, &["key", "id", "name"])
        .ok_or("ispmanager_database_user_form_unreadable")?;
    let form = session
        .read("db.users.edit", &[("elid", &user_key), ("plid", &db_key)])
        .map_err(|_| "ispmanager_database_user_form_unreadable")?;
    let user = text(&form, "username")
        .filter(|value| !value.is_empty())
        .ok_or("ispmanager_database_username_missing")?;
    let panel =
        text(&form, "password").filter(|value| !value.is_empty() && !masked_password(value));
    let (password, source) = match panel {
        Some(password) => (password, PasswordSource::Panel),
        None => match manual(&user).filter(|value| !value.is_empty()) {
            Some(password) => (password, PasswordSource::Manual),
            None => {
                return Err(DatabaseAccessError {
                    code: "ispmanager_database_password_hidden",
                    user: Some(user),
                })
            }
        },
    };
    let port = text(&db, "server_hostandport").and_then(|value| {
        value
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
    });
    Ok((
        ispmanager_sftp::MysqlAccess {
            database: database.to_owned(),
            user,
            password,
            host: text(&db, "server_host").unwrap_or_else(|| "localhost".to_owned()),
            port,
        },
        source,
    ))
}

/// The names of the MySQL databases of the account, for the Scenario editor.
pub fn database_names(session: &mut Session) -> Result<Vec<String>> {
    let doc = session.read("db", &[])?;
    let mut names: Vec<String> = elements(&doc)
        .into_iter()
        .filter(|db| {
            text(db, "server").is_some_and(|server| server.to_ascii_lowercase().contains("mysql"))
        })
        .filter_map(|db| text(db, "name"))
        .collect();
    names.sort();
    Ok(names)
}

/// The names of the DNS zones (`domain`) or mail domains (`emaildomain`)
/// of the account, for the Scenario editor (D-027 section 10).
pub fn object_names(session: &mut Session, category: &str) -> Result<Vec<String>> {
    let func = match category {
        "dns" => "domain",
        "mail" => "emaildomain",
        "sites" => "webdomain",
        _ => return Err(IspError::NotAllowed(category.to_owned())),
    };
    let doc = session.read(func, &[])?;
    let mut names: Vec<String> = elements(&doc)
        .into_iter()
        .filter_map(|item| text(item, "name"))
        .filter(|name| crate::registry::domain_name(name))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The listed fields of one element as a JSON object of strings. A field
/// that the element does not have is left out.
fn pick(item: &serde_json::Map<String, Value>, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|field| {
                text(item, field).map(|value| ((*field).to_owned(), Value::String(value)))
            })
            .collect(),
    )
}

/// Sorts objects by their JSON text, so that the same data gives the same
/// bytes in each run.
fn sorted(mut items: Vec<Value>) -> Vec<Value> {
    items.sort_by_cached_key(Value::to_string);
    items
}

/// The record type from the panel text, for example `MX` from
/// `MX (mail server)`.
fn record_type(rtype: &str) -> String {
    rtype
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
}

/// A BIND zone file from the JSON of `export_object("dns", ...)` (D-027
/// section 7c). Names are absolute. The SOA record is a comment: the panel
/// gives only its primary name server, and a new server makes its own SOA.
/// A record type that the file cannot write safely is a comment too.
pub fn dns_zone_file(json: &[u8]) -> Option<Vec<u8>> {
    let value: Value = serde_json::from_slice(json).ok()?;
    let zone = value["zone"]["name"].as_str()?;
    let field = |record: &Value, name: &str| record[name].as_str().unwrap_or_default().to_owned();
    let absolute = |name: String| {
        if name.ends_with('.') {
            name
        } else {
            format!("{name}.")
        }
    };
    // A record owner is inside the zone: `@`, an empty name, or a relative
    // name joins the zone. A target can be outside the zone, so only the
    // final dot is added to it.
    let owner = |name: String| {
        let bare = name.trim_end_matches('.');
        let lower = bare.to_ascii_lowercase();
        let zone_lower = zone.to_ascii_lowercase();
        if bare.is_empty() || bare == "@" {
            format!("{zone}.")
        } else if lower == zone_lower || lower.ends_with(&format!(".{zone_lower}")) {
            format!("{bare}.")
        } else if name.ends_with('.') {
            name
        } else {
            format!("{bare}.{zone}.")
        }
    };
    let mut text = format!(
        "; Zone {zone} from ISPmanager (domain, domain.record).\n\
         ; QADREN writes the records of the panel list; MX and SRV values come\n\
         ; from the record form. The SOA is a comment only.\n\
         $ORIGIN {zone}.\n"
    );
    for record in value["records"].as_array()? {
        let kind = record_type(&field(record, "rtype"));
        let name = owner(field(record, "name"));
        let ttl = field(record, "ttl");
        let ttl = if ttl.chars().all(|ch| ch.is_ascii_digit()) {
            ttl
        } else {
            String::new()
        };
        let value = field(record, "value");
        let data = match kind.as_str() {
            "A" | "AAAA" => Some(value.clone()),
            "NS" | "CNAME" | "PTR" => Some(absolute(value.clone())),
            "MX" => Some(format!(
                "{} {}",
                field(record, "priority"),
                absolute(value.clone())
            )),
            "SRV" => Some(format!(
                "{} {} {} {}",
                field(record, "priority"),
                field(record, "weight"),
                field(record, "port"),
                absolute(value.clone())
            )),
            "TXT" => Some(
                txt_parts(&value)
                    .into_iter()
                    .map(|part| format!("\"{}\"", part.replace('\\', "\\\\").replace('"', "\\\"")))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };
        let comment = data.is_none() || data.as_deref().is_some_and(|data| data.contains('\n'));
        let line = format!("{name}\t{ttl}\tIN\t{kind}\t{}", data.unwrap_or(value));
        if comment {
            text.push_str("; ");
        }
        text.push_str(&line.replace('\n', " "));
        text.push('\n');
    }
    Some(text.into_bytes())
}

/// TXT text in parts of at most 255 bytes. A part never ends inside a
/// UTF-8 character.
fn txt_parts(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = value;
    while !rest.is_empty() {
        let mut end = rest.len().min(255);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&rest[..end]);
        rest = &rest[end..];
    }
    parts
}

/// What a site file holds besides the site settings (D-027 section 7c).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SiteOptions {
    /// The certificates whose `webdomains` name the site.
    pub ssl: bool,
    /// The FTP users whose home folder is the site root or inside it.
    pub ftp: bool,
}

const SSL_FIELDS: &[&str] = &[
    "name",
    "type",
    "info",
    "valid_after",
    "webdomains",
    "emaildomains",
    "is_letsencrypt",
    "letsencrypt",
    "mail_ssl",
    "active",
];

/// True when a comma or space separated domain list names the site or its
/// `www.` name.
fn names_site(domains: &str, site: &str) -> bool {
    let www = format!("www.{site}");
    domains
        .split([',', ' ', ';'])
        .map(str::trim)
        .any(|domain| domain.eq_ignore_ascii_case(site) || domain.eq_ignore_ascii_case(&www))
}

/// True when an FTP home folder belongs to the site: the site root or a
/// folder inside it, or a folder of the site above its root whose path has
/// the site name (the panel root is `/www/<site>/web`, the FTP home is often
/// `/www/<site>`). A shared folder such as `/` or `/www` does not belong to
/// one site. Both paths are as the panel gives them, from the account home.
fn home_in_site(home: &str, docroot: &str, site: &str) -> bool {
    let home = home.trim_end_matches('/');
    let root = docroot.trim_end_matches('/');
    if home.is_empty() || root.is_empty() {
        return false;
    }
    let inside = |path: &str, base: &str| path == base || path.starts_with(&format!("{base}/"));
    inside(home, root) || (inside(root, home) && home.split('/').any(|part| part == site))
}

/// One site of the account for the Scenario editor: its root folder and
/// its database, if the panel names one (D-027 section 7c).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteOverview {
    pub name: String,
    pub docroot: Option<String>,
    pub database: Option<String>,
}

pub fn sites_overview(session: &mut Session) -> Result<Vec<SiteOverview>> {
    let doc = session.read("webdomain", &[])?;
    let mut sites: Vec<SiteOverview> = elements(&doc)
        .into_iter()
        .filter_map(|site| {
            let name = text(site, "name").filter(|name| crate::registry::domain_name(name))?;
            Some(SiteOverview {
                name,
                // Absolute, or relative to the account home: the editor
                // resolves a relative root against the home folder.
                docroot: text(site, "docroot")
                    .filter(|root| !root.is_empty() && !root.split('/').any(|part| part == "..")),
                database: text(site, "database").filter(|db| !db.is_empty()),
            })
        })
        .collect();
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    sites.dedup_by(|a, b| a.name == b.name);
    Ok(sites)
}

/// The export of one Backup/export object from the panel (D-027 section
/// 10): `cron` (`all`), `dns` (a zone), or `mail` (a mail domain). It reads
/// list functions only and takes the listed fields, without internal keys.
/// The result is pretty JSON with sorted keys and sorted lists.
pub fn export_object(
    session: &mut Session,
    category: &str,
    provider_id: &str,
    site: SiteOptions,
) -> std::result::Result<Vec<u8>, &'static str> {
    let read = |session: &mut Session, func: &str, params: &[(&str, &str)]| {
        session.read(func, params).map_err(|error| match error {
            IspError::Panel(kind) if kind == "access" => "ispmanager_permission_denied",
            IspError::Panel(kind) if kind == "missed" => "ispmanager_function_missing",
            _ => "ispmanager_network_error",
        })
    };
    let value = match category {
        "cron" => {
            let doc = read(session, "scheduler", &[])?;
            let jobs = elements(&doc)
                .into_iter()
                .map(|job| {
                    pick(
                        job,
                        &[
                            "command",
                            "interval",
                            "schedule_type",
                            "active",
                            "description",
                        ],
                    )
                })
                .collect();
            serde_json::json!({
                "format": "qadren-ispmanager-cron/1",
                "source": "scheduler",
                "jobs": sorted(jobs),
            })
        }
        "dns" => {
            let doc = read(session, "domain", &[])?;
            let zone = elements(&doc)
                .into_iter()
                .find(|zone| text(zone, "name").as_deref() == Some(provider_id))
                .ok_or("ispmanager_object_missing")?;
            let zone = pick(zone, &["name", "dtype", "dnssecstatus"]);
            let list = read(session, "domain.record", &[("elid", provider_id)])?;
            let mut records = Vec::new();
            for record in elements(&list) {
                let mut item = pick(record, &["name", "rtype", "ttl", "value"]);
                // The list has no MX or SRV priority, weight, or port: the
                // record form has them. It is opened without `sok`.
                let kind = record_type(&text(record, "rtype").unwrap_or_default());
                if matches!(kind.as_str(), "MX" | "SRV") {
                    let rkey = text(record, "rkey").ok_or("ispmanager_dns_record_unreadable")?;
                    let form = read(
                        session,
                        "domain.record.edit",
                        &[("elid", &rkey), ("plid", provider_id)],
                    )?;
                    if let (Value::Object(item), Value::Object(extra)) =
                        (&mut item, pick(&form, &["priority", "weight", "port"]))
                    {
                        item.extend(extra);
                    }
                }
                records.push(item);
            }
            serde_json::json!({
                "format": "qadren-ispmanager-dns/2",
                "source": "domain, domain.record, domain.record.edit (MX and SRV)",
                "zone": zone,
                "records": sorted(records),
            })
        }
        "mail" => {
            let doc = read(session, "emaildomain", &[])?;
            let domain = elements(&doc)
                .into_iter()
                .find(|domain| text(domain, "name").as_deref() == Some(provider_id))
                .ok_or("ispmanager_object_missing")?;
            let domain = pick(
                domain,
                &[
                    "name",
                    "active",
                    "dkim",
                    "dmarc",
                    "spamassassin",
                    "defaction",
                    "secure",
                ],
            );
            let suffix = format!("@{provider_id}");
            let boxes = read(session, "email", &[])?;
            // `size_used` changes with each letter; it is not a setting.
            let mut mailboxes = Vec::new();
            for mailbox in elements(&boxes)
                .into_iter()
                .filter(|mailbox| text(mailbox, "name").is_some_and(|name| name.ends_with(&suffix)))
            {
                let mut entry = pick(
                    mailbox,
                    &[
                        "name",
                        "active",
                        "aliases",
                        "dontsave",
                        "responder",
                        "spamassassin",
                        "size_total",
                    ],
                );
                // The form opened without `sok` (owner decision 2026-11-14).
                // It also has `passwd`, which is never taken.
                if let Some(name) = text(mailbox, "name") {
                    let form = read(session, "email.edit", &[("elid", &name)])?;
                    if let (Value::Object(map), Value::Object(extra)) =
                        (&mut entry, pick(&form, &["forward", "maxsize", "note"]))
                    {
                        map.extend(extra);
                    }
                }
                mailboxes.push(entry);
            }
            serde_json::json!({
                "format": "qadren-ispmanager-mail/2",
                "source": "emaildomain, email, email.edit",
                "note": "Passwords and letters are not in this file.",
                "domain": domain,
                "mailboxes": sorted(mailboxes),
            })
        }
        "sites" => {
            let site_options = site;
            let doc = read(session, "webdomain", &[])?;
            let site = elements(&doc)
                .into_iter()
                .find(|site| text(site, "name").as_deref() == Some(provider_id))
                .ok_or("ispmanager_object_missing")?;
            let docroot = text(site, "docroot");
            let mut value = serde_json::json!({
                "format": "qadren-ispmanager-site/1",
                "source": "webdomain",
                "site": pick(site, &[
                    "name", "active", "docroot", "public_docroot", "ipaddr", "handler",
                    "php", "php_mode", "php_version", "secure", "database", "git",
                    "cgi_site_settings",
                ]),
            });
            if site_options.ssl {
                let doc = read(session, "sslcert", &[])?;
                let certificates = elements(&doc)
                    .into_iter()
                    .filter(|cert| {
                        text(cert, "webdomains")
                            .is_some_and(|domains| names_site(&domains, provider_id))
                    })
                    .map(|cert| pick(cert, SSL_FIELDS))
                    .collect();
                value["certificates"] = Value::Array(sorted(certificates));
            }
            if site_options.ftp {
                let doc = read(session, "ftp.user", &[])?;
                let users = elements(&doc)
                    .into_iter()
                    .filter(|user| {
                        text(user, "home").is_some_and(|home| {
                            docroot
                                .as_deref()
                                .is_some_and(|root| home_in_site(&home, root, provider_id))
                        })
                    })
                    .map(|user| pick(user, &["name", "home", "active"]))
                    .collect();
                value["ftp_users"] = Value::Array(sorted(users));
            }
            value
        }
        "ssl" => {
            let doc = read(session, "sslcert", &[])?;
            // The list has no private key; `key` is the list key of the panel.
            let certificates = elements(&doc)
                .into_iter()
                .map(|cert| pick(cert, SSL_FIELDS))
                .collect();
            serde_json::json!({
                "format": "qadren-ispmanager-ssl/1",
                "source": "sslcert",
                "note": "Certificate list only. Private keys are not in this file.",
                "certificates": sorted(certificates),
            })
        }
        "ftp" => {
            let doc = read(session, "ftp.user", &[])?;
            let users = elements(&doc)
                .into_iter()
                .map(|user| pick(user, &["name", "home", "active"]))
                .collect();
            serde_json::json!({
                "format": "qadren-ispmanager-ftp/1",
                "source": "ftp.user",
                "note": "Passwords are not in this file.",
                "users": sorted(users),
            })
        }
        // The limits of the account plan. The current use changes with
        // each run, so it is not in the file (owner decision 2026-11-14).
        "accounts" => {
            let doc = read(session, "userstat", &[])?;
            let limits = elements(&doc)
                .into_iter()
                .map(|limit| pick(limit, &["name", "units", "usages_total"]))
                .collect();
            serde_json::json!({
                "format": "qadren-ispmanager-accounts/1",
                "source": "userstat",
                "note": "Plan limits only. The current use is not in this file.",
                "limits": sorted(limits),
            })
        }
        // The PHP versions and, for each version, its extensions (owner
        // decision 2026-11-14). `phpextensions` reads one version as `elid`.
        "php" => {
            let doc = read(session, "phpversions", &[])?;
            let mut versions = Vec::new();
            for version in elements(&doc) {
                let mut entry = pick(version, &["name", "state", "apache", "cgi"]);
                if let Some(key) = text(version, "key").filter(|key| !key.is_empty()) {
                    let list = read(session, "phpextensions", &[("elid", key.as_str())])?;
                    let extensions = elements(&list)
                        .into_iter()
                        .map(|extension| pick(extension, &["name", "enabled", "compiled"]))
                        .collect();
                    if let Value::Object(map) = &mut entry {
                        map.insert("extensions".to_owned(), Value::Array(sorted(extensions)));
                    }
                }
                versions.push(entry);
            }
            serde_json::json!({
                "format": "qadren-ispmanager-php/1",
                "source": "phpversions, phpextensions",
                "versions": sorted(versions),
            })
        }
        _ => return Err("invalid_selection"),
    };
    let mut bytes = serde_json::to_vec_pretty(&value).map_err(|_| "invalid_request")?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// The certificate and its chain of each certificate of the account, as
/// one PEM file for each certificate name (owner decision 2026-11-14). The
/// form `sslcert.edit` is opened without `sok`. Its private key field
/// `crtkey` is never taken (D-027 section 9). A certificate without `crt`,
/// for example a pending request, gets no file.
pub fn ssl_certificate_files(
    session: &mut Session,
) -> std::result::Result<Vec<(String, Vec<u8>)>, &'static str> {
    let read = |session: &mut Session, func: &str, params: &[(&str, &str)]| {
        session.read(func, params).map_err(|error| match error {
            IspError::Panel(kind) if kind == "access" => "ispmanager_permission_denied",
            IspError::Panel(kind) if kind == "missed" => "ispmanager_function_missing",
            _ => "ispmanager_network_error",
        })
    };
    let doc = read(session, "sslcert", &[])?;
    let certificates: Vec<(String, String)> = elements(&doc)
        .into_iter()
        .filter_map(|cert| Some((text(cert, "name")?, text(cert, "key")?)))
        .collect();
    let mut files = Vec::new();
    for (name, key) in certificates {
        let form = read(session, "sslcert.edit", &[("elid", &key)])?;
        let Some(crt) =
            text(&form, "crt").filter(|crt| crt.contains("-----BEGIN CERTIFICATE-----"))
        else {
            continue;
        };
        let chain = text(&form, "cacrt").unwrap_or_default();
        // `all` is the provider ID of the whole list.
        if !crate::registry::domain_name(&name)
            || name == "all"
            || crt.contains("PRIVATE KEY")
            || chain.contains("PRIVATE KEY")
        {
            return Err("invalid_request");
        }
        let mut pem = crt.trim().to_owned();
        pem.push('\n');
        if !chain.trim().is_empty() {
            pem.push_str(chain.trim());
            pem.push('\n');
        }
        files.push((name, pem.into_bytes()));
    }
    files.sort();
    files.dedup_by(|a, b| a.0 == b.0);
    Ok(files)
}

/// The result of one connection check. `status` is a Registry status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub status: &'static str,
    pub error: Option<String>,
    pub categories: Vec<CategoryReport>,
    pub file_transport: Option<FileTransport>,
    pub databases: Option<DatabaseProbe>,
    pub export_probe: Option<ExportProbe>,
}

fn category_state(functions: &[FunctionReport]) -> CapabilityState {
    if functions
        .iter()
        .any(|report| matches!(report.result, "items" | "empty"))
    {
        CapabilityState::VerifiedMetadataOnly
    } else if !functions.is_empty() && functions.iter().all(|report| report.result == "denied") {
        CapabilityState::PermissionDenied
    } else if !functions.is_empty() && functions.iter().all(|report| report.result == "missing") {
        CapabilityState::Unsupported
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

fn function_report(function: &'static str, outcome: Result<ListOutcome>) -> FunctionReport {
    match outcome {
        Ok(ListOutcome::Items { fields }) => FunctionReport {
            function,
            result: "items",
            error: None,
            fields: Some(fields),
        },
        Ok(ListOutcome::Empty) => FunctionReport {
            function,
            result: "empty",
            error: None,
            fields: None,
        },
        Err(error) => FunctionReport {
            function,
            result: match &error {
                IspError::Panel(kind) if kind == "access" => "denied",
                IspError::Panel(kind) if kind == "missed" => "missing",
                _ => "error",
            },
            error: Some(error.to_string()),
            fields: None,
        },
    }
}

/// The records of the first DNS zone, read with `elid`, and the form of
/// its first MX record without `sok` (D-027 section 7c).
fn probe_dns_records(session: &mut Session) -> (Option<FunctionReport>, Option<Vec<String>>) {
    let Some(zone) = session
        .read("domain", &[])
        .ok()
        .and_then(|zones| elements(&zones).into_iter().next().cloned())
    else {
        return (None, None);
    };
    let Some((_, key)) = key_field(&zone, &["name", "key", "id"]) else {
        return (None, None);
    };
    let records = session.read("domain.record", &[("elid", &key)]);
    let form = records.as_ref().ok().and_then(|doc| {
        let mx = elements(doc)
            .into_iter()
            .find(|record| text(record, "rtype").is_some_and(|kind| kind.starts_with("MX")))?
            .clone();
        let rkey = text(&mx, "rkey")?;
        let form = session
            .read("domain.record.edit", &[("elid", &rkey), ("plid", &key)])
            .ok()?;
        Some(sorted_fields(&form))
    });
    let outcome = records.map(|doc| classify_list(&doc));
    (Some(function_report("domain.record", outcome)), form)
}

/// The CAA, TLSA, and DS records of the first DNS zone (D-027 section 7c.6):
/// the parts of the list value and the form fields of the first record of
/// each type. It keeps no value.
fn probe_dns_extra_records(session: &mut Session) -> Vec<DnsRecordProbe> {
    let Some(zone) = session
        .read("domain", &[])
        .ok()
        .and_then(|zones| elements(&zones).into_iter().next().cloned())
    else {
        return Vec::new();
    };
    let Some((_, key)) = key_field(&zone, &["name", "key", "id"]) else {
        return Vec::new();
    };
    let Ok(records) = session.read("domain.record", &[("elid", &key)]) else {
        return Vec::new();
    };
    let mut probes = Vec::new();
    for kind in ["CAA", "TLSA", "DS"] {
        let Some(record) = elements(&records)
            .into_iter()
            .find(|record| text(record, "rtype").is_some_and(|rtype| record_type(&rtype) == kind))
            .cloned()
        else {
            continue;
        };
        let form_fields = text(&record, "rkey").and_then(|rkey| {
            session
                .read("domain.record.edit", &[("elid", &rkey), ("plid", &key)])
                .ok()
                .map(|form| sorted_fields(&form))
        });
        probes.push(DnsRecordProbe {
            rtype: kind.to_owned(),
            value_parts: text(&record, "value").map_or(0, |value| value.split_whitespace().count()),
            form_fields,
        });
    }
    probes
}

/// The form and the filters of the first mailbox (D-027 section 7c.6).
fn probe_mailbox_form(session: &mut Session) -> (Option<Vec<String>>, Option<FunctionReport>) {
    let Some(name) = session.read("email", &[]).ok().and_then(|boxes| {
        elements(&boxes)
            .into_iter()
            .find_map(|mailbox| text(mailbox, "name"))
    }) else {
        return (None, None);
    };
    let form = session
        .read("email.edit", &[("elid", &name)])
        .ok()
        .map(|form| sorted_fields(&form));
    let sorter = session
        .read("email.sorter", &[("elid", &name)])
        .map(|doc| classify_list(&doc));
    (form, Some(function_report("email.sorter", sorter)))
}

/// The form of the first certificate (D-027 sections 7c and 9). The values
/// stay in memory for this check only.
fn probe_ssl_form(session: &mut Session) -> Option<SslFormProbe> {
    let key = session.read("sslcert", &[]).ok().and_then(|certs| {
        elements(&certs)
            .into_iter()
            .find_map(|cert| text(cert, "key"))
    })?;
    let form = session.read("sslcert.edit", &[("elid", &key)]).ok()?;
    let holding = |needle: &str| {
        let mut fields: Vec<String> = form
            .keys()
            .filter(|field| text(&form, field).is_some_and(|value| value.contains(needle)))
            .cloned()
            .collect();
        fields.sort();
        fields
    };
    Some(SslFormProbe {
        fields: sorted_fields(&form),
        certificate_fields: holding("-----BEGIN CERTIFICATE-----"),
        private_key_fields: holding("PRIVATE KEY"),
    })
}

/// The form of mailbox names in the `email` list (D-027 section 7c).
fn probe_mailbox_names(session: &mut Session) -> Option<MailboxNameProbe> {
    let domains: Vec<String> = elements(&session.read("emaildomain", &[]).ok()?)
        .into_iter()
        .filter_map(|domain| text(domain, "name"))
        .collect();
    let boxes = session.read("email", &[]).ok()?;
    let boxes = elements(&boxes);
    let mut probe = MailboxNameProbe {
        mailboxes: boxes.len(),
        ..MailboxNameProbe::default()
    };
    for mailbox in &boxes {
        let name = text(mailbox, "name").unwrap_or_default();
        probe.names_with_at += usize::from(name.contains('@'));
        probe.names_with_domain += usize::from(
            domains
                .iter()
                .any(|domain| name.ends_with(&format!("@{domain}"))),
        );
        for field in mailbox.keys() {
            if text(mailbox, field).is_some_and(|value| domains.contains(&value))
                && !probe.domain_fields.contains(field)
            {
                probe.domain_fields.push(field.clone());
            }
        }
    }
    probe.domain_fields.sort();
    if let Some(first) = domains.first() {
        for (param, slot) in [
            ("elid", &mut probe.with_elid),
            ("plid", &mut probe.with_plid),
        ] {
            *slot = session
                .read("email", &[(param, first.as_str())])
                .ok()
                .map(|doc| elements(&doc).len());
        }
    }
    Some(probe)
}

/// Whether the account home has mail in Maildir form, through SFTP. It
/// reads folder names only and keeps no name.
fn probe_mail_storage(connection: &ispmanager_sftp::SftpConnection, probe: &mut ExportProbe) {
    let Ok(home) = connection.home() else {
        return;
    };
    let mail = format!("{}/email", home.trim_end_matches('/'));
    let folders = |path: &str| -> Vec<String> {
        connection
            .read_dir(path)
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|(_, meta)| meta.kind == "dir")
                    .map(|(name, _)| name)
                    .collect()
            })
            .unwrap_or_default()
    };
    probe.mail_folder =
        Some(matches!(connection.lstat(&mail), Ok(Some(meta)) if meta.kind == "dir"));
    if probe.mail_folder != Some(true) {
        return;
    }
    let maildir = folders(&mail).into_iter().take(5).any(|domain| {
        folders(&format!("{mail}/{domain}"))
            .into_iter()
            .take(5)
            .any(|box_name| {
                // The panel keeps the Maildir in `.maildir` of the mailbox
                // folder (found on 2026-11-14); the folder itself is also
                // checked.
                let is_maildir = |path: String| {
                    let inner = folders(&path);
                    ["cur", "new", "tmp"]
                        .iter()
                        .all(|part| inner.iter().any(|name| name == part))
                };
                let mailbox = format!("{mail}/{domain}/{box_name}");
                is_maildir(format!("{mailbox}/.maildir")) || is_maildir(mailbox)
            })
    });
    probe.maildir = Some(maildir);
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
        databases: None,
        export_probe: None,
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
                .map(|function| function_report(function, session.list(function)))
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
    let (mut databases, access) = probe_databases(&mut session);
    let (dns_records, dns_record_form) = probe_dns_records(&mut session);
    let mut export_probe = ExportProbe {
        dns_records,
        dns_record_form,
        mailbox_names: probe_mailbox_names(&mut session),
        ..ExportProbe::default()
    };
    (export_probe.mailbox_form, export_probe.mailbox_sorter) = probe_mailbox_form(&mut session);
    export_probe.ssl_form = probe_ssl_form(&mut session);
    export_probe.dns_extra_records = probe_dns_extra_records(&mut session);
    if let (SftpCheck::Ready, Some(key)) = (&file_transport, sftp.host_key_sha256) {
        if let Ok(connection) =
            ispmanager_sftp::SftpConnection::open(&host, sftp.port, login, password, key)
        {
            databases.tools = ispmanager_sftp::DumpTool::ALL
                .iter()
                .map(|tool| ToolVersion {
                    tool: *tool,
                    version: connection.tool_version(*tool),
                })
                .collect();
            let has_mysqldump = databases.tools.iter().any(|tool| {
                tool.tool == ispmanager_sftp::DumpTool::Mysqldump && tool.version.is_some()
            });
            if let (true, Some(access)) = (has_mysqldump, &access) {
                databases.mysql_login = Some(connection.mysql_login_check(access));
            }
            probe_mail_storage(&connection, &mut export_probe);
        }
    }
    CheckReport {
        status: "connected",
        error: None,
        categories,
        file_transport: Some(file_transport.into()),
        databases: Some(databases),
        export_probe: Some(export_probe),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(records: Value) -> String {
        let json = serde_json::json!({"zone": {"name": "example.test"}, "records": records});
        String::from_utf8(dns_zone_file(json.to_string().as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn zone_file_puts_every_owner_inside_the_zone() {
        let text = zone(serde_json::json!([
            {"name": "@", "rtype": "A", "ttl": "300", "value": "192.0.2.1"},
            {"name": "www", "rtype": "CNAME", "ttl": "300", "value": "example.test."},
            {"name": "mail.example.test.", "rtype": "A", "ttl": "300", "value": "192.0.2.2"},
            {"name": "ftp.example.test", "rtype": "A", "ttl": "x", "value": "192.0.2.3"},
            {"name": "example.test.", "rtype": "MX (mail)", "ttl": "300", "value": "mail.example.test",
             "priority": "10"},
        ]));
        assert!(text.contains("example.test.\t300\tIN\tA\t192.0.2.1\n"));
        assert!(text.contains("www.example.test.\t300\tIN\tCNAME\texample.test.\n"));
        assert!(text.contains("mail.example.test.\t300\tIN\tA\t192.0.2.2\n"));
        // A TTL that is not a number is left out.
        assert!(text.contains("ftp.example.test.\t\tIN\tA\t192.0.2.3\n"));
        assert!(text.contains("example.test.\t300\tIN\tMX\t10 mail.example.test.\n"));
    }

    #[test]
    fn zone_file_comments_out_a_record_it_cannot_write() {
        let text = zone(serde_json::json!([
            {"name": "@", "rtype": "SOA", "ttl": "300", "value": "ns1.example.test."},
            {"name": "@", "rtype": "CAA", "ttl": "300", "value": "0 issue \"ca.test\""},
        ]));
        assert!(text.contains("; example.test.\t300\tIN\tSOA\t"));
        assert!(text.contains("; example.test.\t300\tIN\tCAA\t"));
    }

    #[test]
    fn txt_parts_split_at_255_bytes_without_breaking_a_character() {
        let ascii = "a".repeat(600);
        let parts = txt_parts(&ascii);
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [255, 255, 90]
        );
        // 254 ASCII bytes, then a two-byte character over the 255-byte edge.
        let text = format!("{}é{}", "a".repeat(254), "b".repeat(10));
        let parts = txt_parts(&text);
        assert_eq!(parts[0].len(), 254);
        assert_eq!(parts.concat(), text);
        let line = zone(serde_json::json!([
            {"name": "@", "rtype": "TXT", "ttl": "300", "value": "say \"hi\" \\ ok"},
        ]));
        assert!(line.contains("IN\tTXT\t\"say \\\"hi\\\" \\\\ ok\"\n"));
    }

    #[test]
    fn masked_password_is_one_repeated_mask_character() {
        assert!(masked_password("********"));
        assert!(masked_password("●●●●"));
        assert!(!masked_password("*a**"));
        assert!(!masked_password("secretvalue"));
        assert!(!masked_password(""));
    }

    #[test]
    fn ftp_home_belongs_to_one_site_only() {
        assert!(home_in_site(
            "/www/example.test",
            "/www/example.test/web",
            "example.test"
        ));
        assert!(home_in_site(
            "/www/example.test/web/up",
            "/www/example.test/web",
            "example.test"
        ));
        assert!(!home_in_site(
            "/www",
            "/www/example.test/web",
            "example.test"
        ));
        assert!(!home_in_site("/", "/www/example.test/web", "example.test"));
        assert!(!home_in_site(
            "/www/other.test",
            "/www/example.test/web",
            "example.test"
        ));
    }

    #[test]
    fn names_site_matches_the_site_and_its_www_name() {
        assert!(names_site("example.test, www.example.test", "example.test"));
        assert!(names_site("WWW.EXAMPLE.TEST", "example.test"));
        assert!(!names_site("sub.example.test", "example.test"));
    }
}
