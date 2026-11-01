//! SFTP file transport of an ISPmanager connection (D-027 section 7a). The
//! client opens only the `sftp` subsystem, never a shell or an `exec`
//! channel, and uses only read operations.
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
        Ok(Ok(sftp)) => Ok((session, sftp)),
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
