//! SFTP file transport of an ISPmanager connection (D-027 section 7a). The
//! client opens the `sftp` subsystem and uses only read operations. It never
//! opens a shell. Its commands are `md5sum` for one file (section 7a.3),
//! `--version` of the database dump tools, `mysqldump` with an optional
//! `gzip -1` pipe, and one `mysql` query of the table list (section 7b).
use russh::client;
use russh::keys::HashAlg;
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 22;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(60);
/// `md5sum` reads the whole file on the server; a large file needs time.
const HASH_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// A large database needs time; a stalled dump ends after this.
const DUMP_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

/// A database dump tool whose version the connection check reads
/// (D-027 section 7b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpTool {
    Mysqldump,
    MariadbDump,
    PgDump,
}

impl DumpTool {
    pub const ALL: [DumpTool; 3] = [DumpTool::Mysqldump, DumpTool::MariadbDump, DumpTool::PgDump];

    fn command(self) -> &'static str {
        match self {
            DumpTool::Mysqldump => "mysqldump",
            DumpTool::MariadbDump => "mariadb-dump",
            DumpTool::PgDump => "pg_dump",
        }
    }
}

#[derive(Debug, Default)]
struct ExecOutput {
    status: Option<u32>,
    stdout_bytes: u64,
    stderr: Vec<u8>,
    /// The last 256 bytes of stderr.
    stderr_tail: Vec<u8>,
}

/// One base table of a MySQL database for the Scenario editor
/// (D-027 section 7b.10). The size and the row count are the estimates of
/// `information_schema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableInfo {
    pub name: String,
    pub bytes: u64,
    pub rows: Option<u64>,
}

/// A table name that QADREN can put in a rule and in a command line: not
/// empty, at most 64 characters, no control character, no `/`, no `\`.
pub fn plain_table_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

fn plain_database(database: &str) -> bool {
    !database.is_empty()
        && !database.starts_with('-')
        && !database.contains(|ch: char| ch.is_control())
}

/// Gives the SQL text of the dump parts to the dump sink and keeps the end
/// of the current part for its completion check.
struct SinkWriter<'a> {
    sink: &'a mut dyn FnMut(&[u8]) -> bool,
    tail: Vec<u8>,
    bytes: u64,
    stopped: bool,
}

impl SinkWriter<'_> {
    fn accept(&mut self, data: &[u8]) -> bool {
        self.tail.extend_from_slice(data);
        if self.tail.len() > 256 {
            self.tail.drain(..self.tail.len() - 256);
        }
        self.bytes += data.len() as u64;
        self.stopped = !(self.sink)(data);
        !self.stopped
    }

    /// True when the text ends with the `-- Dump completed` line.
    fn completed(&self) -> bool {
        String::from_utf8_lossy(&self.tail)
            .trim_end()
            .ends_with("-- Dump completed")
    }
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The MySQL access facts of one database from the panel. They stay in
/// memory; `Debug` shows none of them.
pub struct MysqlAccess {
    pub database: String,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: Option<u16>,
}

impl std::fmt::Debug for MysqlAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MysqlAccess(..)")
    }
}

impl MysqlAccess {
    /// A `[client]` option file. Values are double-quoted with `\` and `"`
    /// escaped, so that `#` and spaces stay part of the value.
    fn option_file(&self) -> Vec<u8> {
        let quote =
            |value: &str| format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
        let mut text = format!(
            "[client]\nuser={}\npassword={}\nhost={}\n",
            quote(&self.user),
            quote(&self.password),
            quote(&self.host)
        );
        if let Some(port) = self.port {
            text.push_str(&format!("port={port}\n"));
        }
        text.into_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MysqlDumpError {
    /// MySQL refused the user or the password from the panel.
    Refused,
    /// The caller stopped the dump.
    Stopped,
    /// No channel, a non-zero exit, a timeout, or an incomplete dump.
    Failed,
}

/// The result of a MySQL sign-in through `mysqldump --no-data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MysqlLoginCheck {
    Ok,
    /// MySQL refused the user or the password.
    Refused,
    Error,
}

/// The result of one SFTP check. Only `Ready` permits SFTP work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpCheck {
    /// The host key matches the confirmed fingerprint, the login is accepted,
    /// and the home folder can be listed.
    Ready,
    /// No fingerprint is confirmed yet. Nothing was sent to the server: the
    /// client disconnects before authentication.
    HostKeyUnconfirmed { fingerprint: String },
    /// The server shows a key that differs from the confirmed one. Nothing
    /// was sent to the server.
    HostKeyChanged { fingerprint: String },
    /// The server uses a host certificate. This stage does not accept one.
    HostCertificateUnsupported,
    /// The server refused the login and the password, for example because
    /// the account has no SSH access.
    AuthRefused,
    /// No TCP connection or no SSH handshake.
    Unreachable(String),
    /// The login is accepted, but the `sftp` subsystem or the listing failed.
    SftpUnavailable(String),
}

/// What the server showed and whether it matched the confirmed key.
#[derive(Default)]
struct HostKeySeen {
    fingerprint: Option<String>,
    certificate: bool,
}

struct Handler {
    expected: Option<String>,
    seen: Arc<Mutex<HostKeySeen>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => {
                let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
                let accepted = self.expected.as_deref() == Some(fingerprint.as_str());
                seen.fingerprint = Some(fingerprint);
                Ok(accepted)
            }
            PublicKeyOrCertificate::Certificate(_) => {
                seen.certificate = true;
                Ok(false)
            }
        }
    }
}

/// One entry of a folder listing. `kind` is `dir`, `file`, `symlink`, or
/// `other` (a device, socket, or pipe). A link is not followed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: &'static str,
    pub size: Option<u64>,
    pub modified: Option<u32>,
    pub mode: Option<u32>,
}

/// The entries of one folder. `path` is the absolute path that the server
/// resolved; `parent` is absent at `/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirListing {
    pub path: String,
    pub parent: Option<String>,
    pub home: String,
    pub entries: Vec<DirEntry>,
    /// The folder has more entries than `MAX_ENTRIES`; the rest is absent.
    pub truncated: bool,
}

/// The largest listing that the browser receives in one response.
pub const MAX_ENTRIES: usize = 20_000;

fn runtime() -> std::result::Result<tokio::runtime::Runtime, SftpCheck> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| SftpCheck::Unreachable(error.to_string()))
}

/// Checks the SFTP transport. Without a confirmed fingerprint the client
/// reads the host key and disconnects before it sends the login.
pub fn check(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    expected: Option<&str>,
) -> SftpCheck {
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(error) => return error,
    };
    runtime.block_on(async {
        match list_async(host, port, login, password, expected, None).await {
            Ok(_) => SftpCheck::Ready,
            Err(error) => error,
        }
    })
}

/// Lists one folder through SFTP. `path` is absolute; `None` is the home
/// folder of the account. The host key must be confirmed: without it the
/// client sends no login.
pub fn list_dir(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    expected: &str,
    path: Option<&str>,
) -> std::result::Result<DirListing, SftpCheck> {
    runtime()?.block_on(list_async(
        host,
        port,
        login,
        password,
        Some(expected),
        path,
    ))
}

async fn list_async(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    expected: Option<&str>,
    path: Option<&str>,
) -> std::result::Result<DirListing, SftpCheck> {
    let (session, sftp) = open(host, port, login, password, expected).await?;
    let listed = tokio::time::timeout(OPERATION_TIMEOUT, read_folder(&sftp, path)).await;
    let _ = sftp.close().await;
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    match listed {
        Ok(result) => result.map_err(SftpCheck::SftpUnavailable),
        Err(_) => Err(SftpCheck::SftpUnavailable("listing timed out".to_owned())),
    }
}

async fn read_folder(
    sftp: &SftpSession,
    path: Option<&str>,
) -> std::result::Result<DirListing, String> {
    let home = sftp
        .canonicalize(".")
        .await
        .map_err(|error| error.to_string())?;
    let path = match path {
        Some(path) if path.starts_with('/') => sftp
            .canonicalize(path)
            .await
            .map_err(|error| error.to_string())?,
        Some(_) => return Err("path is not absolute".to_owned()),
        None => home.clone(),
    };
    let mut entries: Vec<DirEntry> = sftp
        .read_dir(path.as_str())
        .await
        .map_err(|error| error.to_string())?
        .filter(|entry| entry.file_name() != "." && entry.file_name() != "..")
        .map(|entry| {
            let metadata = entry.metadata();
            let kind = match entry.file_type() {
                FileType::Dir => "dir",
                FileType::File => "file",
                FileType::Symlink => "symlink",
                FileType::Other => "other",
            };
            DirEntry {
                name: entry.file_name(),
                kind,
                size: metadata.size,
                modified: metadata.mtime,
                mode: metadata.permissions,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        (a.kind != "dir", a.name.to_lowercase()).cmp(&(b.kind != "dir", b.name.to_lowercase()))
    });
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    let parent = (path != "/").then(|| match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_owned(),
        Some((parent, _)) => parent.to_owned(),
    });
    Ok(DirListing {
        path,
        parent,
        home,
        entries,
        truncated,
    })
}

/// Connects, checks the host key, signs in, and opens the `sftp` subsystem.
async fn open(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    expected: Option<&str>,
) -> std::result::Result<(client::Handle<Handler>, SftpSession), SftpCheck> {
    let seen = Arc::new(Mutex::new(HostKeySeen::default()));
    let handler = Handler {
        expected: expected.map(ToOwned::to_owned),
        seen: Arc::clone(&seen),
    };
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(OPERATION_TIMEOUT),
        ..Default::default()
    });
    let connected = tokio::time::timeout(
        CONNECT_TIMEOUT,
        client::connect(config, (host, port), handler),
    )
    .await;
    let mut session = match connected {
        Ok(Ok(session)) => session,
        Ok(Err(error)) => return Err(refused_host_key(&seen, expected, error)),
        Err(_) => return Err(SftpCheck::Unreachable("connection timed out".to_owned())),
    };
    match tokio::time::timeout(
        OPERATION_TIMEOUT,
        session.authenticate_password(login, password),
    )
    .await
    {
        Ok(Ok(result)) if result.success() => {}
        Ok(Ok(_)) => return Err(SftpCheck::AuthRefused),
        Ok(Err(error)) => return Err(SftpCheck::Unreachable(error.to_string())),
        Err(_) => return Err(SftpCheck::Unreachable("sign-in timed out".to_owned())),
    }
    let opened = tokio::time::timeout(OPERATION_TIMEOUT, async {
        let channel = session
            .channel_open_session()
            .await
            .map_err(|error| error.to_string())?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| error.to_string())?;
        SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| error.to_string())
    })
    .await;
    match opened {
        Ok(Ok(sftp)) => {
            // russh-sftp stops each request after 10 s by default. Read-ahead
            // queues several reads, so on a busy link one answer can wait
            // longer while bytes still flow (found on 2026-11-05). Use the
            // same limit as every other SFTP operation.
            sftp.set_timeout(OPERATION_TIMEOUT.as_secs());
            Ok((session, sftp))
        }
        Ok(Err(error)) => Err(SftpCheck::SftpUnavailable(error)),
        Err(_) => Err(SftpCheck::SftpUnavailable(
            "sftp start timed out".to_owned(),
        )),
    }
}

/// A rejected host key ends the handshake with an error. The key that the
/// server showed tells which case it is.
fn refused_host_key(
    seen: &Mutex<HostKeySeen>,
    expected: Option<&str>,
    error: russh::Error,
) -> SftpCheck {
    let seen = seen.lock().unwrap_or_else(|poison| poison.into_inner());
    if seen.certificate {
        return SftpCheck::HostCertificateUnsupported;
    }
    match (&seen.fingerprint, expected) {
        (Some(fingerprint), None) => SftpCheck::HostKeyUnconfirmed {
            fingerprint: fingerprint.clone(),
        },
        (Some(fingerprint), Some(expected)) if fingerprint != expected => {
            SftpCheck::HostKeyChanged {
                fingerprint: fingerprint.clone(),
            }
        }
        _ => SftpCheck::Unreachable(error.to_string()),
    }
}

/// Why a read on an open SFTP session failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpReadError {
    NotFound,
    PermissionDenied,
    /// A network, protocol, or server error. It can pass later.
    Failed(String),
}

fn read_error(error: russh_sftp::client::error::Error) -> SftpReadError {
    use russh_sftp::client::error::Error;
    use russh_sftp::protocol::StatusCode;
    match &error {
        Error::Status(status) if status.status_code == StatusCode::NoSuchFile => {
            SftpReadError::NotFound
        }
        Error::Status(status) if status.status_code == StatusCode::PermissionDenied => {
            SftpReadError::PermissionDenied
        }
        _ => SftpReadError::Failed(error.to_string()),
    }
}

/// The type, size, and modification time of one server object. A link is
/// not followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMeta {
    pub kind: &'static str,
    pub size: u64,
    pub modified: Option<u32>,
}

impl ObjectMeta {
    /// The planner revision of a server file: SFTP gives no checksum, so the
    /// size and the modification time identify a version.
    pub fn revision(&self) -> String {
        format!("sftp:{}:{}", self.size, self.modified.unwrap_or(0))
    }
}

fn kind_of(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Dir => "dir",
        FileType::File => "file",
        FileType::Symlink => "symlink",
        FileType::Other => "other",
    }
}

/// One open, authenticated SFTP session for the Worker: a walk step or one
/// file effect. It uses only read operations and the one allowed hash
/// command, and closes on drop.
pub struct SftpConnection {
    runtime: tokio::runtime::Runtime,
    session: Option<client::Handle<Handler>>,
    /// `None` only during drop. The session objects must drop inside the
    /// runtime context: outside it, tokio panics ("no reactor running").
    sftp: Option<SftpSession>,
}

impl SftpConnection {
    /// The host key must be the confirmed fingerprint.
    pub fn open(
        host: &str,
        port: u16,
        login: &str,
        password: &str,
        expected: &str,
    ) -> std::result::Result<Self, SftpCheck> {
        let runtime = runtime()?;
        let (session, sftp) =
            runtime.block_on(open(host, port, login, password, Some(expected)))?;
        Ok(Self {
            runtime,
            session: Some(session),
            sftp: Some(sftp),
        })
    }

    fn sftp(&self) -> std::result::Result<&SftpSession, SftpReadError> {
        self.sftp
            .as_ref()
            .ok_or_else(|| SftpReadError::Failed("session closed".to_owned()))
    }

    /// The home folder of the account: the source root of a Backup/export
    /// Scenario (D-027 section 6).
    pub fn home(&self) -> std::result::Result<String, SftpReadError> {
        // A tokio timer needs the runtime context when it is made.
        let _context = self.runtime.enter();
        match self.runtime.block_on(tokio::time::timeout(
            OPERATION_TIMEOUT,
            self.sftp()?.canonicalize("."),
        )) {
            Ok(Ok(home)) => Ok(home),
            Ok(Err(error)) => Err(read_error(error)),
            Err(_) => Err(SftpReadError::Failed("home lookup timed out".to_owned())),
        }
    }

    /// `None` when the object does not exist.
    pub fn lstat(&self, path: &str) -> std::result::Result<Option<ObjectMeta>, SftpReadError> {
        // A tokio timer needs the runtime context when it is made.
        let _context = self.runtime.enter();
        let result = self.runtime.block_on(tokio::time::timeout(
            OPERATION_TIMEOUT,
            self.sftp()?.symlink_metadata(path),
        ));
        match result {
            Ok(Ok(metadata)) => Ok(Some(ObjectMeta {
                kind: kind_of(metadata.file_type()),
                size: metadata.size.unwrap_or(0),
                modified: metadata.mtime,
            })),
            Ok(Err(error)) => match read_error(error) {
                SftpReadError::NotFound => Ok(None),
                other => Err(other),
            },
            Err(_) => Err(SftpReadError::Failed("stat timed out".to_owned())),
        }
    }

    /// All entries of one folder, without `.` and `..`.
    pub fn read_dir(
        &self,
        path: &str,
    ) -> std::result::Result<Vec<(String, ObjectMeta)>, SftpReadError> {
        // A tokio timer needs the runtime context when it is made.
        let _context = self.runtime.enter();
        let result = self.runtime.block_on(tokio::time::timeout(
            OPERATION_TIMEOUT,
            self.sftp()?.read_dir(path),
        ));
        match result {
            Ok(Ok(entries)) => Ok(entries
                .map(|entry| {
                    let metadata = entry.metadata();
                    (
                        entry.file_name(),
                        ObjectMeta {
                            kind: kind_of(entry.file_type()),
                            size: metadata.size.unwrap_or(0),
                            modified: metadata.mtime,
                        },
                    )
                })
                .collect()),
            Ok(Err(error)) => Err(read_error(error)),
            Err(_) => Err(SftpReadError::Failed("listing timed out".to_owned())),
        }
    }

    /// Opens one file for reading. The reader is a blocking `Read`.
    pub fn open_read(&self, path: &str) -> std::result::Result<SftpReader<'_>, SftpReadError> {
        // A tokio timer needs the runtime context when it is made.
        let _context = self.runtime.enter();
        let file = self
            .runtime
            .block_on(tokio::time::timeout(
                OPERATION_TIMEOUT,
                self.sftp()?.open(path),
            ))
            .map_err(|_| SftpReadError::Failed("open timed out".to_owned()))?
            .map_err(read_error)?;
        Ok(SftpReader {
            connection: self,
            file: Some(file),
        })
    }

    /// The MD5 of one server file from the one allowed command, `md5sum`
    /// (D-027 section 7a.3). `None` when the account has no shell, the
    /// command fails, or the answer is not one MD5 for this path. The caller
    /// then reads the file through SFTP.
    pub fn remote_md5(&self, path: &str) -> Option<String> {
        if !path.starts_with('/') || path.contains(|ch: char| ch.is_control()) {
            return None;
        }
        let stdout = self.exec(format!("md5sum -- {}", shell_quote(path)), HASH_TIMEOUT)?;
        let text = String::from_utf8(stdout).ok()?;
        let line = text.strip_suffix('\n')?;
        if line.contains('\n') {
            return None;
        }
        let (md5, rest) = line.split_once("  ")?;
        let valid = md5.len() == 32 && md5.bytes().all(|byte| byte.is_ascii_hexdigit());
        (valid && rest == path).then(|| md5.to_ascii_lowercase())
    }

    /// The first line of `<tool> --version`, or `None` when the tool is
    /// absent or the account has no shell (D-027 section 7b). The command
    /// takes no argument from the user.
    pub fn tool_version(&self, tool: DumpTool) -> Option<String> {
        let stdout = self.exec(format!("{} --version", tool.command()), OPERATION_TIMEOUT)?;
        let line = String::from_utf8(stdout)
            .ok()?
            .lines()
            .next()?
            .trim()
            .to_owned();
        let printable = !line.is_empty() && line.chars().all(|ch| !ch.is_control());
        printable.then(|| line.chars().take(200).collect())
    }

    /// Runs one command of the compiled list on an exec channel. `None`
    /// when the channel or the command fails, or stdout exceeds 64 KiB.
    fn exec(&self, command: String, timeout: Duration) -> Option<Vec<u8>> {
        let mut stdout = Vec::new();
        let mut overflow = false;
        let output = self.exec_streaming(command, &[], timeout, |data| {
            stdout.extend_from_slice(data);
            overflow |= stdout.len() > 64 * 1024;
            !overflow
        })?;
        (output.status == Some(0) && !overflow).then_some(stdout)
    }

    /// Runs one command, writes `stdin` and closes it, and gives stdout to
    /// `sink` in chunks. `sink` returns false to stop. Stderr keeps its
    /// first 4 KiB. `None` when the channel cannot open or time runs out.
    fn exec_streaming(
        &self,
        command: String,
        stdin: &[u8],
        timeout: Duration,
        mut sink: impl FnMut(&[u8]) -> bool,
    ) -> Option<ExecOutput> {
        // A tokio timer needs the runtime context when it is made.
        let _context = self.runtime.enter();
        let session = self.session.as_ref()?;
        let output = self.runtime.block_on(tokio::time::timeout(timeout, async {
            let mut channel = session.channel_open_session().await.ok()?;
            channel.exec(true, command).await.ok()?;
            if !stdin.is_empty() {
                channel.data(stdin).await.ok()?;
            }
            channel.eof().await.ok()?;
            let mut output = ExecOutput::default();
            while let Some(message) = channel.wait().await {
                match message {
                    russh::ChannelMsg::Data { data } => {
                        output.stdout_bytes += data.len() as u64;
                        if !sink(&data) {
                            let _ = channel.close().await;
                            return Some(output);
                        }
                    }
                    russh::ChannelMsg::ExtendedData { data, .. } => {
                        let room = 4096_usize.saturating_sub(output.stderr.len());
                        output
                            .stderr
                            .extend_from_slice(&data[..data.len().min(room)]);
                        output.stderr_tail.extend_from_slice(&data);
                        let extra = output.stderr_tail.len().saturating_sub(256);
                        output.stderr_tail.drain(..extra);
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => {
                        output.status = Some(exit_status)
                    }
                    russh::ChannelMsg::Close => break,
                    _ => {}
                }
            }
            Some(output)
        }));
        output.ok()?
    }

    /// Signs in to MySQL with `mysqldump --no-data` for one database
    /// (D-027 section 7b). It reads the table structure only, takes no
    /// lock, and discards the output. The access facts travel on stdin as
    /// an option file, never in the command line.
    pub fn mysql_login_check(&self, access: &MysqlAccess) -> MysqlLoginCheck {
        let command = format!(
            "mysqldump --defaults-extra-file=/dev/stdin --no-data --single-transaction \
             --skip-lock-tables --no-tablespaces --set-gtid-purged=OFF {}",
            shell_quote(&access.database)
        );
        // A name that starts with `-` would be read as an option.
        if access.database.is_empty()
            || access.database.starts_with('-')
            || access.database.contains(|ch: char| ch.is_control())
        {
            return MysqlLoginCheck::Error;
        }
        let Some(output) =
            self.exec_streaming(command, &access.option_file(), OPERATION_TIMEOUT, |_| true)
        else {
            return MysqlLoginCheck::Error;
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        match output.status {
            Some(0) if output.stdout_bytes > 0 => MysqlLoginCheck::Ok,
            _ if stderr.contains("Access denied") => MysqlLoginCheck::Refused,
            _ => MysqlLoginCheck::Error,
        }
    }

    /// True when the account shell can pipe through `gzip -1`. The probe
    /// compresses one fixed word and checks it on the Mac, so a missing
    /// `gzip` or a shell without `{ }` gives false (D-027 section 7b.6).
    pub fn gzip_pipe_available(&self) -> bool {
        use std::io::Read;
        let Some(stdout) = self.exec(
            "{ echo qadren; echo qadren-exit=0 >&2; } | gzip -1".to_owned(),
            OPERATION_TIMEOUT,
        ) else {
            return false;
        };
        let mut text = String::new();
        flate2::read::GzDecoder::new(stdout.as_slice())
            .read_to_string(&mut text)
            .is_ok()
            && text == "qadren\n"
    }

    /// The base tables of one MySQL database with their estimated sizes,
    /// from one read-only `information_schema` query through `mysql`
    /// (D-027 section 7b.10). A table whose name QADREN cannot put in a
    /// rule is left out.
    pub fn mysql_tables(
        &self,
        access: &MysqlAccess,
    ) -> std::result::Result<Vec<TableInfo>, MysqlDumpError> {
        if !plain_database(&access.database) {
            return Err(MysqlDumpError::Failed);
        }
        let query = "SELECT TABLE_NAME, COALESCE(DATA_LENGTH,0)+COALESCE(INDEX_LENGTH,0), \
                     TABLE_ROWS FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' \
                     ORDER BY TABLE_NAME";
        let command = format!(
            "mysql --defaults-extra-file=/dev/stdin --batch --skip-column-names \
             --default-character-set=utf8mb4 -e {} {}",
            shell_quote(query),
            shell_quote(&access.database)
        );
        let mut stdout = Vec::new();
        let mut overflow = false;
        let output = self
            .exec_streaming(command, &access.option_file(), OPERATION_TIMEOUT, |data| {
                stdout.extend_from_slice(data);
                overflow |= stdout.len() > 4 * 1024 * 1024;
                !overflow
            })
            .ok_or(MysqlDumpError::Failed)?;
        if String::from_utf8_lossy(&output.stderr).contains("Access denied") {
            return Err(MysqlDumpError::Refused);
        }
        if output.status != Some(0) || overflow {
            return Err(MysqlDumpError::Failed);
        }
        let text = String::from_utf8(stdout).map_err(|_| MysqlDumpError::Failed)?;
        Ok(text
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let name = fields.next()?;
                let bytes = fields.next()?.parse().ok()?;
                let rows = fields.next().and_then(|value| value.parse().ok());
                plain_table_name(name).then(|| TableInfo {
                    name: name.to_owned(),
                    bytes,
                    rows,
                })
            })
            .collect())
    }

    /// Dumps one MySQL database through `mysqldump` (D-027 section 7b) and
    /// gives the SQL text to `sink` in chunks. `sink` returns false to stop.
    /// The dump is consistent for InnoDB tables, takes no table lock, and
    /// has no dump date, so the same data gives the same bytes. Each part
    /// is complete only when `mysqldump` exits with 0 and the text ends with
    /// its `-- Dump completed` line.
    ///
    /// The first part has the data of all tables except `structure_only`,
    /// and the routines, triggers, and events. A second part has only the
    /// structure of the `structure_only` tables (section 7b.10). With
    /// `compress`, the server pipes each part through `gzip -1` and the Mac
    /// decompresses it; the gzip check sum is verified (section 7b.6).
    /// Returns the SQL bytes and the bytes that came over the network.
    pub fn mysql_dump(
        &self,
        access: &MysqlAccess,
        structure_only: &[String],
        compress: bool,
        mut sink: impl FnMut(&[u8]) -> bool,
    ) -> std::result::Result<(u64, u64), MysqlDumpError> {
        if !plain_database(&access.database)
            || !structure_only.iter().all(|name| plain_table_name(name))
        {
            return Err(MysqlDumpError::Failed);
        }
        let database = shell_quote(&access.database);
        let mut data = String::from(
            "mysqldump --defaults-extra-file=/dev/stdin --single-transaction --quick \
             --skip-lock-tables --no-tablespaces --set-gtid-purged=OFF --routines --triggers \
             --events --hex-blob --default-character-set=utf8mb4 --skip-dump-date",
        );
        for table in structure_only {
            data.push_str(&format!(
                " --ignore-table={}",
                shell_quote(&format!("{}.{table}", access.database))
            ));
        }
        data.push_str(&format!(" {database}"));
        let mut parts = vec![data];
        if !structure_only.is_empty() {
            let mut structure = format!(
                "mysqldump --defaults-extra-file=/dev/stdin --no-data --single-transaction \
                 --skip-lock-tables --no-tablespaces --set-gtid-purged=OFF --triggers \
                 --default-character-set=utf8mb4 --skip-dump-date {database}"
            );
            for table in structure_only {
                structure.push(' ');
                structure.push_str(&shell_quote(table));
            }
            parts.push(structure);
        }
        let mut writer = SinkWriter {
            sink: &mut sink,
            tail: Vec::new(),
            bytes: 0,
            stopped: false,
        };
        let mut network = 0_u64;
        for command in parts {
            writer.tail.clear();
            network += self.dump_part(access, command, compress, &mut writer)?;
        }
        Ok((writer.bytes, network))
    }

    /// Runs one `mysqldump` command into `writer` and checks its end.
    fn dump_part(
        &self,
        access: &MysqlAccess,
        command: String,
        compress: bool,
        writer: &mut SinkWriter<'_>,
    ) -> std::result::Result<u64, MysqlDumpError> {
        use std::io::Write;
        // The pipe gives the exit status of gzip, so the exit status of
        // mysqldump goes to stderr as the last line.
        let command = if compress {
            format!("{{ {command}; echo qadren-exit=$? >&2; }} | gzip -1")
        } else {
            command
        };
        let mut decoder = compress.then(|| flate2::write::GzDecoder::new(Vec::new()));
        let mut broken = false;
        let output = self
            .exec_streaming(command, &access.option_file(), DUMP_TIMEOUT, |data| {
                match decoder.as_mut() {
                    Some(decoder) => {
                        broken = decoder.write_all(data).is_err();
                        let text = std::mem::take(decoder.get_mut());
                        !broken && writer.accept(&text)
                    }
                    None => writer.accept(data),
                }
            })
            .ok_or(MysqlDumpError::Failed)?;
        if writer.stopped {
            return Err(MysqlDumpError::Stopped);
        }
        if String::from_utf8_lossy(&output.stderr).contains("Access denied") {
            return Err(MysqlDumpError::Refused);
        }
        let exited = match decoder {
            Some(mut decoder) => {
                // `try_finish` verifies the gzip check sum and length.
                let finished = !broken && decoder.try_finish().is_ok();
                let text = std::mem::take(decoder.get_mut());
                let accepted = writer.accept(&text);
                if writer.stopped {
                    return Err(MysqlDumpError::Stopped);
                }
                let tail = String::from_utf8_lossy(&output.stderr_tail);
                finished
                    && accepted
                    && output.status == Some(0)
                    && tail.trim_end().ends_with("qadren-exit=0")
            }
            None => output.status == Some(0),
        };
        if exited && writer.completed() {
            Ok(output.stdout_bytes)
        } else {
            Err(MysqlDumpError::Failed)
        }
    }

    /// Reads a file one time and returns its byte count and MD5.
    pub fn md5(&self, path: &str) -> std::result::Result<(u64, String), SftpReadError> {
        let mut reader = self.open_read(path)?;
        let mut context = md5::Context::new();
        let mut buffer = vec![0_u8; 256 * 1024];
        let mut total = 0_u64;
        loop {
            let count = std::io::Read::read(&mut reader, &mut buffer)
                .map_err(|error| SftpReadError::Failed(error.to_string()))?;
            if count == 0 {
                break;
            }
            context.consume(&buffer[..count]);
            total += count as u64;
        }
        Ok((total, format!("{:x}", context.compute())))
    }
}

impl Drop for SftpConnection {
    fn drop(&mut self) {
        let sftp = self.sftp.take();
        let session = self.session.take();
        self.runtime.block_on(async move {
            if let Some(sftp) = &sftp {
                let _ = sftp.close().await;
            }
            if let Some(session) = &session {
                let _ = session
                    .disconnect(russh::Disconnect::ByApplication, "", "")
                    .await;
            }
            drop(sftp);
            drop(session);
        });
    }
}

/// A blocking reader over one open SFTP file.
pub struct SftpReader<'a> {
    connection: &'a SftpConnection,
    /// `None` only during drop; it closes inside the runtime context.
    file: Option<russh_sftp::client::fs::File>,
}

impl Drop for SftpReader<'_> {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            self.connection.runtime.block_on(async move {
                drop(file);
            });
        }
    }
}

impl std::io::Read for SftpReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use tokio::io::AsyncReadExt;
        let _context = self.connection.runtime.enter();
        let Some(file) = self.file.as_mut() else {
            return Ok(0);
        };
        match self
            .connection
            .runtime
            .block_on(tokio::time::timeout(OPERATION_TIMEOUT, file.read(buffer)))
        {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "sftp read timed out",
            )),
        }
    }
}
