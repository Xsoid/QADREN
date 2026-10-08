//! Read-only Yandex Disk metadata adapter. No content download or write endpoints.
use crate::{AdapterCapabilities, StorageAdapter};
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
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
}
impl From<reqwest::Error> for YandexError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err)
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

pub struct YandexMetadataClient {
    http: Client,
    api_base: String,
}

impl StorageAdapter for YandexMetadataClient {
    fn name(&self) -> &'static str {
        "yandex_disk"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            metadata: true,
            read_content: false,
            write_content: false,
        }
    }
}

impl YandexMetadataClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: Client::builder().timeout(Duration::from_secs(15)).build()?,
            api_base: API.to_owned(),
        })
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

    /// Disk quota metadata; never accesses file bytes.
    pub fn disk_info(&self, token: &str) -> Result<DiskInfo> {
        self.get_json("/", token, &[])
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
            ("limit", limit.clamp(1, 100).to_string()),
            ("offset", offset.to_string()),
        ];
        self.get_json("/resources", token, &query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "HTTP/1.1 {status} Mock\\r\\nContent-Type: application/json\\r\\nContent-Length: {}\\r\\nConnection: close\\r\\n\\r\\n{}",
                payload.len(), payload
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let client = YandexMetadataClient {
            http: Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            api_base: format!("http://{addr}/v1/disk"),
        };
        (client, thread)
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
    fn metadata_capabilities_exclude_contents() {
        let client = YandexMetadataClient::new().unwrap();
        let caps = client.capabilities();
        assert!(caps.metadata);
        assert!(!caps.read_content);
        assert!(!caps.write_content);
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
