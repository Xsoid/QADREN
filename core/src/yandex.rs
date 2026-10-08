//! Read-only Yandex Disk metadata adapter. No content download or write endpoints.
use crate::{AdapterCapabilities, StorageAdapter};
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::StatusCode;
use serde::Deserialize;
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DiskInfo {
    pub total_space: u64,
    pub used_space: u64,
    #[serde(default)]
    pub trash_size: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub md5: Option<String>,
    pub revision: Option<i64>,
    #[serde(rename = "_embedded")]
    pub embedded: Option<ResourceList>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ResourceList {
    pub items: Vec<Resource>,
    pub limit: u32,
    pub offset: u64,
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
