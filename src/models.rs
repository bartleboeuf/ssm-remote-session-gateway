use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::error;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Preferences {
    pub port_range_start: u16,
    pub port_range_end: u16,
    pub logging_level: String,
    pub logging_format: String,
    pub last_profile: Option<String>,
    pub last_region: Option<String>,
    pub ssh_key_folder: Option<String>,
    pub ssh_options: String,
    pub cache_ttl_secs: u64,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            port_range_start: 61000,
            port_range_end: 61100,
            logging_level: "INFO".into(),
            logging_format: "%Y-%m-%dT%H:%M:%S%.3f %l %t".into(),
            last_profile: None,
            last_region: None,
            ssh_key_folder: None,
            ssh_options: "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null".into(),
            cache_ttl_secs: 3600,
        }
    }
}

impl Preferences {
    pub fn path() -> PathBuf {
        let mut dir = directories::ProjectDirs::from("", "", "ssm-remote-session-gateway")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
        let _ = std::fs::create_dir_all(&dir);
        dir.push("preferences.json");
        dir
    }

    pub fn log_dir() -> PathBuf {
        let dir = directories::ProjectDirs::from("", "", "ssm-remote-session-gateway")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(p) = serde_json::from_str(&data) {
                return p;
            }
        }
        Preferences::default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Ok(data) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(path, data) {
                error!("failed to save preferences: {e}");
            }
        }
    }

    pub fn cache_path() -> PathBuf {
        let mut dir = directories::ProjectDirs::from("", "", "ssm-remote-session-gateway")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
        let _ = std::fs::create_dir_all(&dir);
        println!("Cache directory: {:?}", dir);
        dir.push("instance_cache.json");
        dir
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConnectionInfo {
    pub id: uuid::Uuid,
    pub profile: String,
    pub region: String,
    pub instance_id: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub remote_host: Option<String>,
    pub command: String,
    pub key_name: Option<String>,
    pub key_path: Option<String>,
}

#[derive(Debug)]
pub struct ManagedConnection {
    pub info: ConnectionInfo,
    pub child: Option<tokio::process::Child>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedInstances {
    pub fetched_at: i64,
    pub entries: Vec<InstanceItem>,
}

#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub status: String,
    #[serde(flatten)]
    pub payload: T,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub status: String,
    pub error: String,
}

pub fn ok<T: Serialize>(payload: T) -> axum::Json<ApiResponse<T>> {
    axum::Json(ApiResponse {
        status: "success".into(),
        payload,
    })
}

pub fn err(
    status: axum::http::StatusCode,
    msg: impl Into<String>,
) -> (axum::http::StatusCode, axum::Json<ErrorResponse>) {
    (
        status,
        axum::Json(ErrorResponse {
            status: "error".into(),
            error: msg.into(),
        }),
    )
}

#[derive(Debug, Deserialize)]
pub struct ConnectRequest {
    pub profile: String,
    pub region: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstanceItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub state: String,
    pub os: String,
    pub has_ssm: bool,
}

#[derive(Debug, Deserialize)]
pub struct StartPortReq {
    pub remote_port: u16,
    pub local_port: Option<u16>,
    pub remote_host: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RdpClientReq {
    pub ip: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WindowsPasswordReq {
    pub pem_key: Option<String>,
    pub key_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PrefsInput {
    pub port_range: Option<PortRange>,
    pub logging: Option<Logging>,
    pub aws: Option<AwsPref>,
    pub ssh_key_folder: Option<String>,
    pub ssh_options: Option<String>,
    pub cache_ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Deserialize)]
pub struct Logging {
    pub level: String,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AwsPref {
    pub profile: Option<String>,
    pub region: Option<String>,
}
