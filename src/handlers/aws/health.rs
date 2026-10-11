use axum::{Json, response::IntoResponse};
use reqwest;
use std::time::Duration;
use which::which;

use crate::state::AppState;
use axum::extract::State;

use super::common::*;
use super::ecs::*;
use super::rds::*;

/// Access probe flags: (credentials_ok, rds_accessible, ecs_accessible).
pub(super) type AccessFlags = (bool, bool, bool);

/// Cache of AWS access probe results keyed by "profile|region", stored with the time they were taken.
pub(super) static ACCESS_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, AccessFlags)>>,
> = std::sync::OnceLock::new();

/// How long access probe results are reused before AWS is queried again.
pub(super) const ACCESS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Probes credentials, RDS and ECS access for a profile and region, reusing results for 60 seconds.
/// Args:
///   profile: AWS profile name from preferences.
///   region: Region from preferences, if set.
/// Returns:
///   Tuple of (credentials_ok, rds_accessible, ecs_accessible).
pub(super) async fn aws_access_flags(profile: &str, region: Option<String>) -> (bool, bool, bool) {
    let key = format!("{}|{}", profile, region.clone().unwrap_or_default());
    let cache =
        ACCESS_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some((at, flags)) = cache.lock().ok().and_then(|c| c.get(&key).copied())
        && at.elapsed() < ACCESS_CACHE_TTL
    {
        return flags;
    }
    let cfg = sdk_config(profile, region.as_deref()).await;
    let configured = is_aws_configured(&cfg).await;
    let (rds, ecs) = if configured {
        (can_access_rds(&cfg).await, can_access_ecs(&cfg).await)
    } else {
        (false, false)
    };
    let flags = (configured, rds, ecs);
    if let Ok(mut c) = cache.lock() {
        c.insert(key, (std::time::Instant::now(), flags));
    }
    flags
}

/// Health check endpoint to verify AWS CLI, session manager plugin, and credentials availability.
/// Returns a JSON object with the status of each component and environment information. This can be used by
/// the frontend to determine if AWS integration is properly set up and provide helpful error messages if not.
/// Note: This function performs basic checks for the presence of AWS CLI and session manager plugin in the
/// system PATH, as well as verifying that AWS credentials can be loaded using the AWS SDK. It also includes
/// the operating system and application version in the response for debugging purposes. In a real implementation,
/// you might want to expand this to include more detailed checks (e.g. specific AWS CLI version, ability to call
/// a simple AWS API, etc.) depending on your application's requirements.
/// Args:
///     None
/// Returns:
///     JSON object with the following structure:
///     {
///     "aws_cli": bool, // true if AWS CLI is found in PATH false otherwise
///     "session_manager_plugin": bool, // true if session manager plugin is found in PATH false otherwise
///     "aws_credentials": bool, // true if AWS credentials can be loaded false otherwise
///     "os": string, // operating system name (e.g. "windows", "linux", "macos")
///     "version": string // application version from Cargo.toml
///     }
/// Raises:
/// None. This function is designed to be resilient and return a comprehensive status report without throwing
/// errors. Any issues with AWS session manager plugin, or credentials will be reflected in the boolean
/// values of the response, allowing the frontend to provide appropriate guidance to the user.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let smp_installed = which("session-manager-plugin").is_ok();
    let smp_version = if smp_installed {
        match tokio::process::Command::new("session-manager-plugin")
            .arg("--version")
            .output()
            .await
        {
            Ok(output) => {
                let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if s.is_empty() {
                    String::from_utf8_lossy(&output.stderr).trim().to_string()
                } else {
                    s
                }
            }
            Err(_) => "unknown".to_string(),
        }
    } else {
        "not installed".to_string()
    };

    let prefs = state.prefs.read().await.clone();
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let (aws_configured, rds_accessible, ecs_accessible) =
        aws_access_flags(&profile, prefs.last_region.clone()).await;

    Json(serde_json::json!({
        "aws_cli": which("aws").is_ok(),
        "session_manager_plugin": smp_installed,
        "session_manager_plugin_version": smp_version,
        "aws_credentials": aws_configured,
        "rds_accessible": rds_accessible,
        "ecs_accessible": ecs_accessible,
        "os": std::env::consts::OS,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Returns the current application version for the frontend and API consumers.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({"version": env!("CARGO_PKG_VERSION")}))
}

/// GET /api/check-update
/// Queries GitHub Releases API and compares the latest tag with CARGO_PKG_VERSION.
/// Always returns 200; network/parse errors silently return has_update=false.
pub async fn check_update() -> impl IntoResponse {
    const REPO: &str = "bartleboeuf/ssm-remote-session-gateway";
    const API: &str = "https://api.github.com/repos";

    let current = env!("CARGO_PKG_VERSION").to_string();
    let url = format!("{}/{}/releases/latest", API, REPO);

    let ua = format!("ssm-remote-session-gateway/{}", env!("CARGO_PKG_VERSION"));

    let silent_err = || {
        Json(crate::models::UpdateInfo {
            has_update: false,
            current_version: current.clone(),
            latest_version: current.clone(),
            release_url: String::new(),
        })
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return silent_err(),
    };

    let resp = match client.get(&url).header("User-Agent", &ua).send().await {
        Ok(r) => r,
        Err(_) => return silent_err(),
    };

    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return silent_err(),
    };

    let tag = json["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    let html_url = json["html_url"].as_str().unwrap_or("").to_string();
    let has_update = semver_gt(&tag, &current);

    Json(crate::models::UpdateInfo {
        has_update,
        current_version: current,
        latest_version: tag,
        release_url: html_url,
    })
}

/// Returns true if version string `a` is strictly greater than `b` as a semver tuple.
pub(super) fn semver_gt(a: &str, b: &str) -> bool {
    fn parse(s: &str) -> (u64, u64, u64) {
        let mut parts = s.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
        (
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
            parts.next().unwrap_or(0),
        )
    }
    !a.is_empty() && parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_compares_numeric_components_not_text() {
        assert!(semver_gt("1.2.10", "1.2.9"));
        assert!(!semver_gt("1.2.9", "1.2.10"));
    }

    #[test]
    fn semver_equal_or_empty_is_not_greater() {
        assert!(!semver_gt("1.2.3", "1.2.3"));
        assert!(!semver_gt("", "1.0.0"));
    }

    #[test]
    fn semver_handles_missing_components() {
        assert!(semver_gt("2", "1.9.9"));
        assert!(!semver_gt("1.2", "1.2.0"));
    }
}
