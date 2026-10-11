use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::error;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Preferences {
    pub port_range_start: u16,
    pub port_range_end: u16,
    pub logging_level: String,
    pub logging_format: String,
    pub last_profile: Option<String>,
    pub last_region: Option<String>,
    /// Profile and region chosen in Settings; applied at startup in preference to the last connection.
    #[serde(default)]
    pub preferred_profile: Option<String>,
    #[serde(default)]
    pub preferred_region: Option<String>,
    pub ssh_key_folder: Option<String>,
    pub ssh_options: String,
    pub cache_ttl_secs: u64,
    pub linux_username: String,
    pub windows_username: String,
    #[serde(default)]
    pub dark_mode: bool,
    /// When true, SSH/SCP verify host keys strictly (StrictHostKeyChecking=yes).
    #[serde(default = "default_true")]
    pub strict_host_key_checking: bool,
}

/// Serde default helper returning `true`.
fn default_true() -> bool {
    true
}

impl Default for Preferences {
    /// Returns the built-in defaults for all preferences.
    fn default() -> Self {
        Self {
            port_range_start: 61000,
            port_range_end: 61100,
            logging_level: "INFO".into(),
            logging_format: "%Y-%m-%dT%H:%M:%S%.3f %l %t".into(),
            last_profile: None,
            last_region: None,
            preferred_profile: None,
            preferred_region: None,
            ssh_key_folder: None,
            ssh_options: "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null".into(),
            cache_ttl_secs: 3600,
            linux_username: "ec2-user".into(),
            windows_username: "Administrator".into(),
            dark_mode: false,
            strict_host_key_checking: true,
        }
    }
}

impl Preferences {
    /// Returns the platform-appropriate config directory.
    fn project_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "ssm-remote-session-gateway")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| {
                directories::UserDirs::new()
                    .map(|d| d.home_dir().to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            })
    }

    /// File path for the persisted preferences JSON.
    pub fn path() -> PathBuf {
        Self::project_dir().join("preferences.json")
    }

    /// Directory where log files should be stored.
    pub fn log_dir() -> PathBuf {
        Self::project_dir()
    }

    /// Reads the stored preferences file if it exists; otherwise returns defaults.
    pub async fn load() -> Self {
        let path = Self::path();
        if let Ok(data) = tokio::fs::read_to_string(&path).await
            && let Ok(p) = serde_json::from_str(&data)
        {
            return p;
        }
        Preferences::default()
    }

    /// Writes the current preferences to disk, creating the directory if needed.
    pub async fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            error!("failed to create preferences directory: {e}");
            return;
        }
        if let Ok(data) = serde_json::to_string_pretty(self)
            && let Err(e) = tokio::fs::write(path, data).await
        {
            error!("failed to save preferences: {e}");
        }
    }

    /// File path for the cached instance metadata.
    pub fn cache_path() -> PathBuf {
        Self::project_cache_dir().join("instance_cache.json")
    }

    /// Returns the directory used for cache data (separate from config).
    pub fn project_cache_dir() -> PathBuf {
        directories::ProjectDirs::from("", "", "ssm-remote-session-gateway")
            .map(|d| d.cache_dir().to_path_buf())
            .unwrap_or_else(|| {
                directories::UserDirs::new()
                    .map(|d| d.home_dir().to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            })
    }

    /// Creates the cache directory if it does not already exist.
    pub async fn ensure_cache_dir() -> std::io::Result<()> {
        tokio::fs::create_dir_all(Self::project_cache_dir()).await
    }
}

/// AWS account identity for a profile, recorded when the profile connects.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub account_id: String,
    pub account_alias: Option<String>,
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
pub struct RdsInstance {
    pub db_instance_identifier: String,
    pub endpoint: String,
    pub port: u16,
    pub engine: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedRdsInstances {
    pub fetched_at: i64,
    pub entries: Vec<RdsInstance>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EcsTask {
    pub task_arn: String,
    pub task_definition: String,
    pub container_name: String,
    pub launch_type: String,
    pub status: String,
    pub cluster_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedEcsTasks {
    pub fetched_at: i64,
    pub entries: Vec<EcsTask>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedInstances {
    pub fetched_at: i64,
    pub entries: Vec<InstanceItem>,
}

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub has_update: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
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

/// Wraps a payload in the standard success envelope used by the API.
pub fn ok<T: Serialize>(payload: T) -> axum::Json<ApiResponse<T>> {
    axum::Json(ApiResponse {
        status: "success".into(),
        payload,
    })
}

/// Builds a standardized error response tuple for Axum handlers.
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

/// Direction of the file transfer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Upload,
    Download,
}

/// Body sent by the frontend to POST /api/transfer/{instance_id}
#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub direction: TransferDirection,
    pub local_path: String,
    pub remote_path: String,
    pub username: Option<String>,
    pub key_path: Option<String>,
}

/// Status enum for an ongoing or finished transfer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Connecting,
    Transferring,
    Completed,
    Failed,
    Cancelled,
}

/// Progress snapshot returned by GET /api/transfer-progress/{transfer_id}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgress {
    pub id: String,
    pub status: TransferStatus,
    pub percent: u8,
    pub speed: String,
    pub eta: String,
    pub error: Option<String>,
}

/// Internal state for a running transfer
#[derive(Debug)]
#[allow(dead_code)]
pub struct ManagedTransfer {
    pub id: uuid::Uuid,
    pub progress: Arc<Mutex<TransferProgress>>,
    pub scp_child: Option<tokio::process::Child>,
    pub tunnel_child: Option<tokio::process::Child>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
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
    pub preferred_aws: Option<AwsPref>,
    pub ssh_key_folder: Option<String>,
    pub ssh_options: Option<String>,
    pub cache_ttl_secs: Option<u64>,
    pub linux_username: Option<String>,
    pub windows_username: Option<String>,
    pub dark_mode: Option<bool>,
    pub strict_host_key_checking: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_host_key_defaults_to_true_for_old_preference_files() {
        // Files written before the setting existed must load with strict checking on.
        let prefs: Preferences = serde_json::from_str("{}").expect("empty prefs parse");
        assert!(prefs.strict_host_key_checking);
        assert_eq!(prefs.linux_username, "ec2-user");
    }

    #[test]
    fn explicit_host_key_setting_is_respected() {
        let prefs: Preferences =
            serde_json::from_str(r#"{"strict_host_key_checking": false}"#).expect("parse");
        assert!(!prefs.strict_host_key_checking);
    }
}
