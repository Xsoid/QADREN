//! SFTP file transport of an ISPmanager connection (D-027 section 7a). The
//! client opens only the `sftp` subsystem, never a shell or an `exec`
//! channel, and uses only read operations.
use russh::client;
use russh::keys::HashAlg;
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
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

/// Checks the SFTP transport. Without a confirmed fingerprint the client
/// reads the host key and disconnects before it sends the login.
pub fn check(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    expected: Option<&str>,
) -> SftpCheck {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return SftpCheck::Unreachable(error.to_string()),
    };
    runtime.block_on(check_async(host, port, login, password, expected))
}

async fn check_async(
    host: &str,
    port: u16,
    login: &str,
    password: &str,
    expected: Option<&str>,
) -> SftpCheck {
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
        Ok(Err(error)) => return refused_host_key(&seen, expected, error),
        Err(_) => return SftpCheck::Unreachable("connection timed out".to_owned()),
    };
    let result = authenticated(&mut session, login, password).await;
    let _ = session
        .disconnect(russh::Disconnect::ByApplication, "", "")
        .await;
    result
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

async fn authenticated(
    session: &mut client::Handle<Handler>,
    login: &str,
    password: &str,
) -> SftpCheck {
    match tokio::time::timeout(
        OPERATION_TIMEOUT,
        session.authenticate_password(login, password),
    )
    .await
    {
        Ok(Ok(result)) if result.success() => {}
        Ok(Ok(_)) => return SftpCheck::AuthRefused,
        Ok(Err(error)) => return SftpCheck::Unreachable(error.to_string()),
        Err(_) => return SftpCheck::Unreachable("sign-in timed out".to_owned()),
    }
    let listed = tokio::time::timeout(OPERATION_TIMEOUT, async {
        let channel = session.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| russh::Error::IO(std::io::Error::other(error.to_string())))?;
        let home = sftp
            .canonicalize(".")
            .await
            .map_err(|error| russh::Error::IO(std::io::Error::other(error.to_string())))?;
        sftp.read_dir(home)
            .await
            .map_err(|error| russh::Error::IO(std::io::Error::other(error.to_string())))?;
        let _ = sftp.close().await;
        Ok::<(), russh::Error>(())
    })
    .await;
    match listed {
        Ok(Ok(())) => SftpCheck::Ready,
        Ok(Err(error)) => SftpCheck::SftpUnavailable(error.to_string()),
        Err(_) => SftpCheck::SftpUnavailable("listing timed out".to_owned()),
    }
}
