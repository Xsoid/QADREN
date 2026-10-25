//! Yandex Disk adapter: metadata, verified content download (D-023 stage 1),
//! and a no-overwrite upload primitive that no Scenario uses yet.
use crate::{AdapterCapabilities, StorageAdapter};
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::time::Duration;

const API: &str = "https://cloud-api.yandex.net/v1/disk";

#[derive(Debug)]
pub enum YandexError {
    Transport(reqwest::Error),
    InvalidToken,
    Unauthorized,
    Forbidden,
    RateLimited,
    NotFound,
    Provider(u16),
    InvalidPath,
    InvalidAccountIdentity,
    UnsafeTransferUrl,
    UploadRejected(u16),
    UploadOutcomeUnknown,
    /// The downloaded byte count or MD5 differs from the provider metadata.
    ContentMismatch,
    /// The folder tree exceeds the snapshot entry or depth limit.
    SnapshotTooLarge,
    /// The API gave a download link, but the storage host does not serve the
    /// content (for example HTTP 500). Observed on 2026-10-24 for some files
    /// after the October 2026 Yandex data-center incident.
    ContentUnavailable(u16),
}
impl From<reqwest::Error> for YandexError {
    fn from(err: reqwest::Error) -> Self {
        // A download link is signed: it must never reach a log or a message.
        Self::Transport(err.without_url())
    }
}
pub type Result<T> = std::result::Result<T, YandexError>;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiskInfo {
    pub total_space: u64,
    pub used_space: u64,
    #[serde(default)]
    pub trash_size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub md5: Option<String>,
    #[serde(default)]
    pub revision: Option<i64>,
    #[serde(rename = "_embedded")]
    pub embedded: Option<ResourceList>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceList {
    pub items: Vec<Resource>,
    #[serde(default)]
    pub limit: u32,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub total: u64,
}

/// A metadata observation can support reconciliation but never proves content delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataMatch {
    MatchesExpectedMetadata,
    Mismatch,
    InsufficientEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedResourceMetadata {
    pub path: String,
    pub size: u64,
    pub md5: String,
}

fn valid_md5(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn compare_resource_metadata(
    expected: &ExpectedResourceMetadata,
    observed: &Resource,
) -> MetadataMatch {
    if expected.path.is_empty() || !valid_md5(&expected.md5) {
        return MetadataMatch::InsufficientEvidence;
    }
    if observed.path != expected.path || observed.kind != "file" {
        return MetadataMatch::Mismatch;
    }
    let (Some(size), Some(md5)) = (observed.size, observed.md5.as_deref()) else {
        return MetadataMatch::InsufficientEvidence;
    };
    if size == expected.size && valid_md5(md5) && md5.eq_ignore_ascii_case(&expected.md5) {
        MetadataMatch::MatchesExpectedMetadata
    } else {
        MetadataMatch::Mismatch
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TransferLink {
    href: String,
    method: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadOutcome {
    /// Provider accepted the upload request. Reconcile resource metadata
    /// separately; this does not establish cloud content integrity.
    Accepted,
    /// Provider queued asynchronous work; result must be reconciled.
    Pending,
}

/// Streams exactly the byte count that was durably bound to an upload attempt.
/// The final source read probes for both early EOF and trailing bytes before
/// exposing the final chunk to HTTP. A mismatch therefore makes the PUT
/// outcome ambiguous instead of allowing a declared size to misrepresent the
/// source stream.
struct ExactSizeReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> ExactSizeReader<R> {
    fn new(inner: R, expected_size: u64) -> Self {
        Self {
            inner,
            remaining: expected_size,
        }
    }
}

impl<R: Read> Read for ExactSizeReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut trailing = [0_u8; 1];
            return match self.inner.read(&mut trailing)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "upload source exceeds declared size",
                )),
            };
        }

        let maximum = usize::try_from(self.remaining.saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(output.len());
        let count = self.inner.read(&mut output[..maximum])?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "upload source ended before declared size",
            ));
        }
        if count as u64 > self.remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "upload source exceeds declared size",
            ));
        }
        if count as u64 == self.remaining {
            let mut trailing = [0_u8; 1];
            if self.inner.read(&mut trailing)? != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "upload source exceeds declared size",
                ));
            }
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

#[derive(Deserialize)]
struct DiskAccountResponse {
    user: DiskAccountUser,
}

#[derive(Deserialize)]
struct DiskAccountUser {
    uid: serde_json::Value,
}

pub struct YandexMetadataClient {
    http: Client,
    /// Content client: no OAuth header, at most one redirect to a Yandex host.
    download_http: Client,
    api_base: String,
}

/// Hosts that may serve a signed download link or its single redirect.
const TRANSFER_HOST_SUFFIXES: [&str; 2] = [".yandex.net", ".yandex.ru"];

fn trusted_transfer_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            TRANSFER_HOST_SUFFIXES
                .iter()
                .any(|suffix| host.ends_with(suffix))
        })
}

fn download_client(timeout: Duration) -> reqwest::Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // The blocking client applies this limit to each body read, so it
        // stops a stalled transfer without limiting a large file.
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 1 || !trusted_transfer_url(attempt.url()) {
                attempt.error("untrusted download redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
}

/// One file in a Yandex Disk folder snapshot. `relative_path` uses `/` and is
/// relative to the snapshot root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub relative_path: String,
    pub size: u64,
    pub md5: String,
    pub modified: Option<String>,
}

/// Maximum folder depth of a snapshot, as for a local tree.
const MAX_SNAPSHOT_DEPTH: usize = 256;

/// Streams one download and computes its MD5. The reader refuses bytes past
/// the expected size, and at end of stream it refuses a short stream or
/// another MD5. A local writer therefore never publishes unverified content.
pub struct YandexDownload {
    body: reqwest::blocking::Response,
    expected_size: u64,
    expected_md5: String,
    received: u64,
    digest: md5::Context,
    verified: bool,
}

impl Read for YandexDownload {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let count = self.body.read(output)?;
        if count == 0 {
            let md5 = format!("{:x}", self.digest.clone().compute());
            if self.received != self.expected_size || md5 != self.expected_md5 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "download differs from the provider size or MD5",
                ));
            }
            self.verified = true;
            return Ok(0);
        }
        self.received = self.received.saturating_add(count as u64);
        if self.received > self.expected_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "download exceeds the provider size",
            ));
        }
        self.digest.consume(&output[..count]);
        Ok(count)
    }
}

impl YandexDownload {
    /// True only after the reader reached the end of a stream with the
    /// provider size and MD5.
    pub fn verified(&self) -> bool {
        self.verified
    }
}

impl StorageAdapter for YandexMetadataClient {
    fn name(&self) -> &'static str {
        "yandex_disk"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            metadata: true,
            read_content: true,
            write_content: false,
        }
    }
}

impl YandexMetadataClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            download_http: download_client(Duration::from_secs(60))?,
            api_base: API.to_owned(),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_test_api_base(api_base: String) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("test HTTP client"),
            download_http: download_client(Duration::from_secs(3)).expect("test HTTP client"),
            api_base,
        }
    }

    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        token: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        if token.is_empty() || token.contains(|ch: char| ch.is_control()) {
            return Err(YandexError::InvalidToken);
        }
        let authorization = HeaderValue::from_str(&format!("OAuth {token}"))
            .map_err(|_| YandexError::InvalidToken)?;
        let response = self
            .http
            .get(format!("{}{}", self.api_base, endpoint))
            .header(AUTHORIZATION, authorization)
            .query(query)
            .send()?;
        let status = response.status();
        match status {
            StatusCode::UNAUTHORIZED => return Err(YandexError::Unauthorized),
            StatusCode::FORBIDDEN => return Err(YandexError::Forbidden),
            StatusCode::TOO_MANY_REQUESTS => return Err(YandexError::RateLimited),
            StatusCode::NOT_FOUND => return Err(YandexError::NotFound),
            _ => {}
        }
        if !status.is_success() {
            return Err(YandexError::Provider(status.as_u16()));
        }
        Ok(response.json()?)
    }

    /// Identifies the account backing this OAuth token using the provider's
    /// disk-info response. Missing or malformed UID always fails closed.
    pub fn account_uid(&self, token: &str) -> Result<String> {
        let response: DiskAccountResponse = self.get_json("/", token, &[])?;
        let uid = match response.user.uid {
            serde_json::Value::String(value) if !value.is_empty() => value,
            serde_json::Value::Number(value) if value.is_u64() => value.to_string(),
            _ => return Err(YandexError::InvalidAccountIdentity),
        };
        Ok(uid)
    }

    /// Disk quota metadata; never accesses file bytes.
    pub fn disk_info(&self, token: &str) -> Result<DiskInfo> {
        self.get_json("/", token, &[])
    }

    /// Reads metadata for one cloud object; never requests file contents.
    /// The returned metadata is an observation, not proof of an earlier write.
    pub fn resource_metadata(&self, token: &str, path: &str) -> Result<Resource> {
        if path.is_empty() || path.contains(|ch: char| ch.is_control()) {
            return Err(YandexError::InvalidPath);
        }
        self.get_json("/resources", token, &[("path", path.to_owned())])
    }

    /// Re-read only provider metadata after a possible upload. A matching
    /// size and provider MD5 can establish the expected remote object
    /// metadata, but this method neither retries an upload nor changes a Job
    /// or its evidence. Incomplete expected identity fails closed before any
    /// network request.
    pub fn reconcile_uploaded_resource(
        &self,
        token: &str,
        expected: &ExpectedResourceMetadata,
    ) -> Result<MetadataMatch> {
        if expected.path.is_empty() || expected.path.contains(|ch: char| ch.is_control()) {
            return Err(YandexError::InvalidPath);
        }
        if !valid_md5(&expected.md5) {
            return Ok(MetadataMatch::InsufficientEvidence);
        }
        let observed = self.resource_metadata(token, &expected.path)?;
        Ok(compare_resource_metadata(expected, &observed))
    }

    /// Upload content supplied by the caller to a new Yandex Disk path.
    /// This operation never opens local files and never downloads file bytes.
    /// The caller must enforce destination consent and source provenance.
    /// A failed PUT after sending bytes may already have changed remote state.
    pub fn upload_from_reader<R: std::io::Read + Send + 'static>(
        &self,
        token: &str,
        path: &str,
        source: R,
        expected_size: u64,
    ) -> Result<UploadOutcome> {
        if !path.starts_with("disk:/")
            || path.contains(|ch: char| ch.is_control())
            || path == "disk:/"
        {
            return Err(YandexError::InvalidPath);
        }
        let link: TransferLink = self.get_json(
            "/resources/upload",
            token,
            &[("path", path.to_owned()), ("overwrite", "false".to_owned())],
        )?;
        if link.method != "PUT" {
            return Err(YandexError::UnsafeTransferUrl);
        }
        let parsed = reqwest::Url::parse(&link.href).map_err(|_| YandexError::UnsafeTransferUrl)?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || parsed.username() != ""
            || parsed.password().is_some()
        {
            return Err(YandexError::UnsafeTransferUrl);
        }
        let mut exact_source = ExactSizeReader::new(source, expected_size);
        // A zero-length HTTP body may never poll its reader, so force the
        // exact-size guard before initiating the PUT in that special case.
        if expected_size == 0 {
            exact_source
                .read(&mut [0_u8; 1])
                .map_err(|_| YandexError::UploadOutcomeUnknown)?;
        }
        let body = reqwest::blocking::Body::sized(exact_source, expected_size);
        let response = self
            .http
            .put(parsed)
            .body(body)
            .send()
            .map_err(|_| YandexError::UploadOutcomeUnknown)?;
        match response.status() {
            StatusCode::CREATED | StatusCode::OK | StatusCode::NO_CONTENT => {
                Ok(UploadOutcome::Accepted)
            }
            StatusCode::ACCEPTED => Ok(UploadOutcome::Pending),
            StatusCode::UNAUTHORIZED => Err(YandexError::Unauthorized),
            StatusCode::FORBIDDEN => Err(YandexError::Forbidden),
            StatusCode::TOO_MANY_REQUESTS => Err(YandexError::RateLimited),
            code if code.is_server_error() => Err(YandexError::UploadOutcomeUnknown),
            code => Err(YandexError::UploadRejected(code.as_u16())),
        }
    }

    /// Lists directory metadata without downloading file contents.
    /// Limit is deliberately capped to avoid large responses and memory usage.
    pub fn list_folder(
        &self,
        token: &str,
        path: &str,
        offset: u64,
        limit: u32,
    ) -> Result<Resource> {
        if path.is_empty() || path.contains(|ch: char| ch.is_control()) {
            return Err(YandexError::InvalidPath);
        }
        let query = [
            ("path", path.to_owned()),
            ("limit", limit.clamp(1, 1000).to_string()),
            ("offset", offset.to_string()),
        ];
        self.get_json("/resources", token, &query)
    }

    /// Opens one file download. The caller streams the result into a local
    /// staging file and checks `verified`. The OAuth header goes only to the
    /// API host, never to the signed link host. The expected size and MD5
    /// come from metadata that the caller read before its durable attempt.
    pub fn open_download(
        &self,
        token: &str,
        path: &str,
        expected_size: u64,
        expected_md5: &str,
    ) -> Result<YandexDownload> {
        if !path.starts_with("disk:/")
            || path.contains(|ch: char| ch.is_control())
            || path == "disk:/"
        {
            return Err(YandexError::InvalidPath);
        }
        if !valid_md5(expected_md5) {
            return Err(YandexError::ContentMismatch);
        }
        let link: TransferLink =
            self.get_json("/resources/download", token, &[("path", path.to_owned())])?;
        if link.method != "GET" {
            return Err(YandexError::UnsafeTransferUrl);
        }
        let parsed = reqwest::Url::parse(&link.href).map_err(|_| YandexError::UnsafeTransferUrl)?;
        if !trusted_transfer_url(&parsed) {
            return Err(YandexError::UnsafeTransferUrl);
        }
        let response = self.download_http.get(parsed).send()?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(YandexError::NotFound),
            StatusCode::TOO_MANY_REQUESTS => return Err(YandexError::RateLimited),
            code if code.is_server_error() => {
                return Err(YandexError::ContentUnavailable(code.as_u16()))
            }
            code => return Err(YandexError::Provider(code.as_u16())),
        }
        if response
            .content_length()
            .is_some_and(|length| length != expected_size)
        {
            return Err(YandexError::ContentMismatch);
        }
        Ok(YandexDownload {
            body: response,
            expected_size,
            expected_md5: expected_md5.to_ascii_lowercase(),
            received: 0,
            digest: md5::Context::new(),
            verified: false,
        })
    }

    /// Lists every file under `root` through bounded metadata pages. Any
    /// error fails the whole snapshot, so a partial tree never makes a plan.
    /// A file without size or a valid MD5 fails closed.
    pub fn list_tree(
        &self,
        token: &str,
        root: &str,
        max_entries: usize,
    ) -> Result<Vec<RemoteFile>> {
        self.list_selection(token, root, &[], max_entries)
    }

    /// Like `list_tree`, but only below the selected objects of a Scenario.
    /// An empty selection is the full root. A selected object that does not
    /// exist now gives no file. A large Disk is then never walked for a
    /// small selection.
    pub fn list_selection(
        &self,
        token: &str,
        root: &str,
        selected: &[String],
        max_entries: usize,
    ) -> Result<Vec<RemoteFile>> {
        if !root.starts_with("disk:/") || root.contains(|ch: char| ch.is_control()) {
            return Err(YandexError::InvalidPath);
        }
        let root = if root == "disk:/" {
            root
        } else {
            root.trim_end_matches('/')
        };
        let join = |relative: &str| {
            if root.ends_with('/') {
                format!("{root}{relative}")
            } else {
                format!("{root}/{relative}")
            }
        };
        let mut pending = Vec::new();
        let mut files = Vec::new();
        let mut entries = 0usize;
        if selected.is_empty() {
            pending.push((String::new(), 0usize));
        }
        for path in selected {
            let segments: Vec<&str> = path.split('/').collect();
            if segments
                .iter()
                .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
                || path.contains(|ch: char| ch.is_control())
            {
                return Err(YandexError::InvalidPath);
            }
            let item = match self.resource_metadata(token, &join(path)) {
                Ok(item) => item,
                Err(YandexError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            entries += 1;
            if entries > max_entries {
                return Err(YandexError::SnapshotTooLarge);
            }
            match item.kind.as_str() {
                "dir" => pending.push((path.clone(), segments.len())),
                "file" => {
                    let (Some(size), Some(md5)) = (item.size, item.md5) else {
                        return Err(YandexError::Provider(200));
                    };
                    if !valid_md5(&md5) {
                        return Err(YandexError::Provider(200));
                    }
                    files.push(RemoteFile {
                        relative_path: path.clone(),
                        size,
                        md5: md5.to_ascii_lowercase(),
                        modified: item.modified,
                    });
                }
                _ => return Err(YandexError::Provider(200)),
            }
        }
        while let Some((relative, depth)) = pending.pop() {
            let folder = match (relative.is_empty(), root.ends_with('/')) {
                (true, _) => root.to_owned(),
                (false, true) => format!("{root}{relative}"),
                (false, false) => format!("{root}/{relative}"),
            };
            let mut offset = 0u64;
            loop {
                let page = self.list_folder(token, &folder, offset, 1000)?;
                if page.kind != "dir" {
                    return Err(YandexError::InvalidPath);
                }
                let Some(list) = page.embedded else {
                    return Err(YandexError::Provider(200));
                };
                let count = list.items.len() as u64;
                for item in list.items {
                    entries += 1;
                    if entries > max_entries {
                        return Err(YandexError::SnapshotTooLarge);
                    }
                    if item.name.is_empty()
                        || item.name.contains('/')
                        || item.name == "."
                        || item.name == ".."
                    {
                        return Err(YandexError::InvalidPath);
                    }
                    let child = if relative.is_empty() {
                        item.name.clone()
                    } else {
                        format!("{relative}/{}", item.name)
                    };
                    match item.kind.as_str() {
                        "dir" => {
                            if depth + 1 > MAX_SNAPSHOT_DEPTH {
                                return Err(YandexError::SnapshotTooLarge);
                            }
                            pending.push((child, depth + 1));
                        }
                        "file" => {
                            let (Some(size), Some(md5)) = (item.size, item.md5) else {
                                return Err(YandexError::Provider(200));
                            };
                            if !valid_md5(&md5) {
                                return Err(YandexError::Provider(200));
                            }
                            files.push(RemoteFile {
                                relative_path: child,
                                size,
                                md5: md5.to_ascii_lowercase(),
                                modified: item.modified,
                            });
                        }
                        _ => return Err(YandexError::Provider(200)),
                    }
                }
                offset += count;
                if count == 0 || offset >= list.total {
                    break;
                }
            }
        }
        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_reader_rejects_short_or_long_source_before_final_chunk() {
        let mut exact = ExactSizeReader::new(std::io::Cursor::new(b"content".to_vec()), 7);
        let mut bytes = Vec::new();
        exact.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"content");

        let mut short = ExactSizeReader::new(std::io::Cursor::new(b"short".to_vec()), 7);
        assert_eq!(short.read(&mut [0_u8; 16]).unwrap(), 5);
        assert_eq!(
            short.read(&mut [0_u8; 16]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );

        let mut long = ExactSizeReader::new(std::io::Cursor::new(b"content+".to_vec()), 7);
        assert_eq!(
            long.read(&mut [0_u8; 16]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let mut zero_with_bytes = ExactSizeReader::new(std::io::Cursor::new(vec![1]), 0);
        assert_eq!(
            zero_with_bytes.read(&mut [0_u8; 1]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    // Local loopback mock: verifies HTTP status mapping and no content transfers.
    fn mocked_client(
        status: u16,
        body: &str,
    ) -> (YandexMetadataClient, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let payload = body.to_owned();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0u8; 8192];
            let n = stream.read(&mut request).unwrap();
            let message = String::from_utf8_lossy(&request[..n]);
            assert!(message.starts_with("GET /v1/disk/ HTTP/1.1"));
            assert!(message.contains("OAuth test-token"));
            let response = format!(
                "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(), payload
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = YandexMetadataClient {
            http: Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            download_http: download_client(Duration::from_secs(3)).unwrap(),
            api_base: format!("http://{addr}/v1/disk"),
        };
        (client, thread)
    }

    #[test]
    fn upload_refuses_insecure_signed_url_without_sending_content() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0u8; 8192];
            let count = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]);
            assert!(request.starts_with("GET /v1/disk/resources/upload?"));
            assert!(request.contains("overwrite=false"));
            let body = r#"{"href":"http://127.0.0.1:1/content","method":"PUT"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = YandexMetadataClient {
            http: Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            download_http: download_client(Duration::from_secs(3)).unwrap(),
            api_base: format!("http://{address}/v1/disk"),
        };
        let result = client.upload_from_reader(
            "test-token",
            "disk:/file.bin",
            std::io::Cursor::new(b"content".to_vec()),
            7,
        );
        assert!(matches!(result, Err(YandexError::UnsafeTransferUrl)));
        server.join().unwrap();
    }

    #[test]
    fn single_resource_metadata_uses_read_only_get() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0u8; 8192];
            let size = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            assert!(request.starts_with("GET /v1/disk/resources?"));
            assert!(request.contains("path=disk%3A%2Fcheck.txt"));
            assert!(request.contains("OAuth test-token"));
            let body = r#"{"name":"check.txt","path":"disk:/check.txt","type":"file","size":5,"modified":"2026-10-09T00:00:00Z","md5":"abcdef0123456789abcdef0123456789"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = YandexMetadataClient {
            http: Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            download_http: download_client(Duration::from_secs(3)).unwrap(),
            api_base: format!("http://{address}/v1/disk"),
        };
        let match_result = client
            .reconcile_uploaded_resource(
                "test-token",
                &ExpectedResourceMetadata {
                    path: "disk:/check.txt".into(),
                    size: 5,
                    md5: "abcdef0123456789abcdef0123456789".into(),
                },
            )
            .unwrap();
        assert_eq!(match_result, MetadataMatch::MatchesExpectedMetadata);
        server.join().unwrap();
    }

    #[test]
    fn folder_page_uses_bounded_metadata_get_with_requested_offset() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0u8; 8192];
            let size = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            assert!(request.starts_with("GET /v1/disk/resources?"));
            assert!(request.contains("path=disk%3A%2FArchive"));
            assert!(request.contains("limit=50"));
            assert!(request.contains("offset=50"));
            assert!(request.contains("OAuth test-token"));
            let body = r#"{"name":"Archive","path":"disk:/Archive","type":"dir","_embedded":{"items":[{"name":"next","path":"disk:/Archive/next","type":"dir"}],"limit":50,"offset":50,"total":101}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = YandexMetadataClient::with_test_api_base(format!("http://{address}/v1/disk"));
        let page = client
            .list_folder("test-token", "disk:/Archive", 50, 50)
            .unwrap();
        server.join().unwrap();
        let listing = page.embedded.unwrap();
        assert_eq!(listing.offset, 50);
        assert_eq!(listing.total, 101);
        assert_eq!(listing.items[0].path, "disk:/Archive/next");
    }

    #[test]
    fn metadata_comparison_fails_closed_on_missing_or_mismatched_fields() {
        let expected = ExpectedResourceMetadata {
            path: "disk:/check.txt".into(),
            size: 5,
            md5: "ABCDEF0123456789ABCDEF0123456789".into(),
        };
        let mut observed = Resource {
            name: "check.txt".into(),
            path: "disk:/check.txt".into(),
            kind: "file".into(),
            size: Some(5),
            modified: None,
            md5: Some("abcdef0123456789abcdef0123456789".into()),
            revision: None,
            embedded: None,
        };
        assert_eq!(
            compare_resource_metadata(&expected, &observed),
            MetadataMatch::MatchesExpectedMetadata
        );
        observed.md5 = None;
        assert_eq!(
            compare_resource_metadata(&expected, &observed),
            MetadataMatch::InsufficientEvidence
        );
        observed.md5 = Some("bad".into());
        assert_eq!(
            compare_resource_metadata(&expected, &observed),
            MetadataMatch::Mismatch
        );
        observed.md5 = Some("abcdef0123456789abcdef0123456789".into());
        observed.path = "disk:/different.txt".into();
        assert_eq!(
            compare_resource_metadata(&expected, &observed),
            MetadataMatch::Mismatch
        );
    }

    #[test]
    fn invalid_resource_path_is_rejected_without_network_access() {
        let client = YandexMetadataClient::new().unwrap();
        assert!(matches!(
            client.resource_metadata("token", ""),
            Err(YandexError::InvalidPath)
        ));
        assert!(matches!(
            client.resource_metadata("token", "disk:/file\n"),
            Err(YandexError::InvalidPath)
        ));
        assert_eq!(
            client
                .reconcile_uploaded_resource(
                    "token",
                    &ExpectedResourceMetadata {
                        path: "disk:/check.txt".into(),
                        size: 5,
                        md5: "not-a-provider-md5".into(),
                    },
                )
                .unwrap(),
            MetadataMatch::InsufficientEvidence
        );
    }

    #[test]
    fn account_uid_is_observed_from_provider_disk_info() {
        let (client, server) = mocked_client(
            200,
            r#"{"user":{"uid":"123456"},"total_space":2048,"used_space":512}"#,
        );
        assert_eq!(client.account_uid("test-token").unwrap(), "123456");
        server.join().unwrap();
    }

    #[test]
    fn absent_account_identity_fails_closed() {
        let (client, server) = mocked_client(
            200,
            r#"{"user":{"uid":null},"total_space":2048,"used_space":512}"#,
        );
        assert!(matches!(
            client.account_uid("test-token"),
            Err(YandexError::InvalidAccountIdentity)
        ));
        server.join().unwrap();
    }

    #[test]
    fn mock_success_reads_quota_metadata() {
        let (client, server) = mocked_client(200, r#"{"total_space":2048,"used_space":512}"#);
        let quota = client.disk_info("test-token").unwrap();
        server.join().unwrap();
        assert_eq!(quota.total_space, 2048);
        assert_eq!(quota.used_space, 512);
    }

    #[test]
    fn mock_maps_authorization_rate_limit_and_server_errors() {
        for (status, expected) in [
            (401, "unauthorized"),
            (403, "forbidden"),
            (429, "rate_limited"),
            (503, "provider"),
        ] {
            let (client, server) = mocked_client(status, r#"{"message":"provider error"}"#);
            let result = client.disk_info("test-token");
            server.join().unwrap();
            let category = match result {
                Err(YandexError::Unauthorized) => "unauthorized",
                Err(YandexError::Forbidden) => "forbidden",
                Err(YandexError::RateLimited) => "rate_limited",
                Err(YandexError::Provider(503)) => "provider",
                other => panic!("unexpected result: {other:?}"),
            };
            assert_eq!(category, expected);
        }
    }

    #[test]
    fn mock_timeout_returns_transport_error_without_retrying_content() {
        use std::io::Read;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            std::thread::sleep(Duration::from_millis(250));
        });
        let client = YandexMetadataClient {
            http: Client::builder()
                .timeout(Duration::from_millis(50))
                .build()
                .unwrap(),
            download_http: download_client(Duration::from_secs(3)).unwrap(),
            api_base: format!("http://{addr}/v1/disk"),
        };
        assert!(matches!(
            client.disk_info("test-token"),
            Err(YandexError::Transport(_))
        ));
        server.join().unwrap();
    }

    #[test]
    fn capabilities_allow_download_but_not_write() {
        let client = YandexMetadataClient::new().unwrap();
        let caps = client.capabilities();
        assert!(caps.metadata);
        assert!(caps.read_content);
        assert!(!caps.write_content);
    }

    #[test]
    fn download_link_must_be_https_on_a_yandex_host() {
        let ok = |url: &str| trusted_transfer_url(&reqwest::Url::parse(url).unwrap());
        assert!(ok("https://downloader.disk.yandex.ru/disk/x"));
        assert!(ok("https://s123.mds.storage.yandex.net/x"));
        assert!(!ok("http://downloader.disk.yandex.ru/disk/x"));
        assert!(!ok("https://yandex.ru.example.com/x"));
        assert!(!ok("https://evilyandex.net/x"));
        assert!(!ok("https://user:pw@downloader.disk.yandex.ru/x"));
    }

    #[test]
    fn parses_disk_info_without_network() {
        let disk: DiskInfo =
            serde_json::from_str(r#"{"total_space":5000,"used_space":123,"trash_size":9}"#)
                .unwrap();
        assert_eq!(disk.total_space, 5000);
        assert_eq!(disk.used_space, 123);
    }

    #[test]
    fn parses_directory_with_file_metadata() {
        let response: Resource = serde_json::from_str(
            r#"{"name":"disk","path":"disk:/","type":"dir","_embedded":{"items":[{"name":"test.txt","path":"disk:/test.txt","type":"file","size":8,"modified":"2026-10-08T12:00:00+00:00","md5":"abc","revision":5}],"limit":20,"offset":0,"total":1}}"#,
        )
        .unwrap();
        let list = response.embedded.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].size, Some(8));
        assert_eq!(list.items[0].md5.as_deref(), Some("abc"));
    }

    #[test]
    fn permits_embedded_resource_list_without_optional_pagination_fields() {
        let resource: Resource = serde_json::from_str(
            r#"{"name":"disk","path":"disk:/","type":"dir","_embedded":{"items":[{"name":"Docs","path":"disk:/Docs","type":"dir"}]}}"#,
        )
        .unwrap();
        let list = resource.embedded.unwrap();
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.total, 0);
    }

    #[test]
    fn rejects_invalid_token_and_paths_before_request() {
        let client = YandexMetadataClient::new().unwrap();
        assert!(matches!(
            client.disk_info("bad\ntoken"),
            Err(YandexError::InvalidToken)
        ));
        assert!(matches!(
            client.list_folder("abc", "", 0, 10),
            Err(YandexError::InvalidPath)
        ));
    }
}
