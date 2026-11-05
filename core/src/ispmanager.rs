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
const OBJECT_READS: [&str; 2] = ["db.users", "db.users.edit"];

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
    let Some(db) = elements(&doc).first().copied().cloned() else {
        return None;
    };
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
    let Some((db_field, db_key)) = key_field(&db, &["pair", "key", "id", "name"]) else {
        return None;
    };
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
    let Some((user_field, user_key)) = key_field(&user, &["key", "id", "name"]) else {
        return None;
    };
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
            probe.password_masked = password.as_ref().map(|value| {
                let mut chars = value.chars();
                let first = chars.next();
                first.is_some_and(|first| "*•●x".contains(first))
                    && chars.all(|ch| Some(ch) == first)
            });
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

/// The MySQL access facts of one selected database, read from the panel at
/// the start of each export (D-027 section 7b). QADREN never stores them.
/// The user is the first user of the database with the read right.
pub fn mysql_access(
    session: &mut Session,
    database: &str,
) -> std::result::Result<ispmanager_sftp::MysqlAccess, &'static str> {
    let doc = session
        .read("db", &[])
        .map_err(|_| "ispmanager_database_list_failed")?;
    let db = elements(&doc)
        .into_iter()
        .find(|db| text(db, "name").as_deref() == Some(database))
        .cloned()
        .ok_or("ispmanager_database_missing")?;
    if !text(&db, "server").is_some_and(|server| server.to_ascii_lowercase().contains("mysql")) {
        return Err("ispmanager_database_unsupported");
    }
    let (_, db_key) = key_field(&db, &["pair", "key", "id", "name"])
        .ok_or("ispmanager_database_access_unavailable")?;
    let users = session
        .read("db.users", &[("elid", &db_key)])
        .map_err(|_| "ispmanager_database_access_unavailable")?;
    let users = elements(&users);
    let readable = |user: &&&serde_json::Map<String, Value>| {
        text(user, "read")
            .is_some_and(|value| matches!(value.as_str(), "on" | "yes" | "1" | "true"))
    };
    let user = users
        .iter()
        .find(readable)
        .or_else(|| users.first())
        .ok_or("ispmanager_database_access_unavailable")?;
    let (_, user_key) =
        key_field(user, &["key", "id", "name"]).ok_or("ispmanager_database_access_unavailable")?;
    let form = session
        .read("db.users.edit", &[("elid", &user_key), ("plid", &db_key)])
        .map_err(|_| "ispmanager_database_access_unavailable")?;
    let password = text(&form, "password")
        .filter(|value| !value.is_empty())
        .ok_or("ispmanager_database_access_unavailable")?;
    let user = text(&form, "username")
        .filter(|value| !value.is_empty())
        .ok_or("ispmanager_database_access_unavailable")?;
    let port = text(&db, "server_hostandport").and_then(|value| {
        value
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
    });
    Ok(ispmanager_sftp::MysqlAccess {
        database: database.to_owned(),
        user,
        password,
        host: text(&db, "server_host").unwrap_or_else(|| "localhost".to_owned()),
        port,
    })
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

/// The result of one connection check. `status` is a Registry status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub status: &'static str,
    pub error: Option<String>,
    pub categories: Vec<CategoryReport>,
    pub file_transport: Option<FileTransport>,
    pub databases: Option<DatabaseProbe>,
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
                            IspError::Panel(kind) if kind == "missed" => "missing",
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
    let (mut databases, access) = probe_databases(&mut session);
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
        }
    }
    CheckReport {
        status: "connected",
        error: None,
        categories,
        file_transport: Some(file_transport.into()),
        databases: Some(databases),
    }
}
