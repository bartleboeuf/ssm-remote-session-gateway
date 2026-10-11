use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_config::meta::region::RegionProviderChain;
use aws_config::retry::RetryConfig;
use aws_config::timeout::TimeoutConfig;
use aws_credential_types::provider::ProvideCredentials;
use aws_types::region::Region;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;

use crate::models::{CachedInstances, InstanceItem};
use crate::state::AppState;
use tracing::{info, warn};

use super::ec2::*;

/// Builds a fully configured AWS SDK loader, allowing overrides for retries and timeouts.
pub(super) async fn load_aws_config(
    profile: &str,
    region: RegionProviderChain,
    retry_config: Option<RetryConfig>,
    timeout_config: Option<TimeoutConfig>,
) -> aws_config::SdkConfig {
    let mut loader = aws_config::from_env()
        .profile_name(profile)
        .behavior_version(BehaviorVersion::latest());

    loader = loader.region(region);
    if let Some(retry_config) = retry_config {
        loader = loader.retry_config(retry_config);
    }
    if let Some(timeout_config) = timeout_config {
        loader = loader.timeout_config(timeout_config);
    }

    loader.load().await
}

/// Creates a RegionProviderChain from an optional region string, falling back to default providers.
pub(super) fn region_chain(region: Option<String>) -> RegionProviderChain {
    if let Some(region) = region {
        RegionProviderChain::first_try(Region::new(region)).or_default_provider()
    } else {
        RegionProviderChain::default_provider()
    }
}

/// Loads the instance cache from disk into memory if the cache is currently empty.
pub async fn load_instance_cache_from_disk(state: &AppState) {
    let need_load = { state.instance_cache.lock().await.is_empty() };
    if !need_load {
        return;
    }

    let path = crate::models::Preferences::cache_path();
    match tokio::fs::read_to_string(&path).await {
        Ok(data) => {
            match serde_json::from_str::<std::collections::HashMap<String, CachedInstances>>(&data)
            {
                Ok(map) => {
                    let mut cache = state.instance_cache.lock().await;
                    if cache.is_empty() {
                        *cache = map;
                        info!("Loaded instance cache from disk");
                    }
                }
                Err(e) => warn!("Failed to parse instance cache file: {e}"),
            }
        }
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!("Failed to read instance cache file: {err}");
            }
        }
    }
}

/// Persists the instance cache map to disk after ensuring the cache directory exists.
pub(super) async fn save_instance_cache(
    cache: &std::collections::HashMap<String, CachedInstances>,
) -> Result<()> {
    crate::models::Preferences::ensure_cache_dir().await?;
    let path = crate::models::Preferences::cache_path();
    let data = serde_json::to_string_pretty(cache)?;
    tokio::fs::write(path, data).await?;
    Ok(())
}

pub(super) struct RefreshSlot {
    pub(super) notify: Arc<Notify>,
    pub(super) leader: bool,
}

/// Claims a refresh slot for the given cache key, preventing concurrent AWS calls.
pub(super) async fn register_instance_refresh(state: &AppState, cache_key: &str) -> RefreshSlot {
    let mut refreshes = state.instance_refreshes.lock().await;
    if let Some(existing) = refreshes.get(cache_key) {
        return RefreshSlot {
            notify: existing.clone(),
            leader: false,
        };
    }

    let notify = Arc::new(Notify::new());
    refreshes.insert(cache_key.to_string(), notify.clone());
    RefreshSlot {
        notify,
        leader: true,
    }
}

/// Releases a refresh slot and notifies waiters that the cache key was refreshed.
pub(super) async fn complete_instance_refresh(state: &AppState, cache_key: &str) {
    let notify = {
        let mut refreshes = state.instance_refreshes.lock().await;
        refreshes.remove(cache_key)
    };

    if let Some(notify) = notify {
        notify.notify_waiters();
    }
}

/// Filters cached instances according to the requested state and OS filters.
pub(super) fn filter_cached_instances(
    entries: Vec<InstanceItem>,
    params: &InstanceListParams,
) -> Vec<InstanceItem> {
    entries
        .into_iter()
        .filter(|i| {
            let state_ok = match &params.filter_state {
                Some(f) if !f.is_empty() && f != "all" => i.state == *f,
                _ => true,
            };
            let os_ok = match &params.filter_os {
                Some(os) if !os.is_empty() && os != "all" => i.os == *os,
                _ => true,
            };
            state_ok && os_ok
        })
        .collect()
}

/// Checks if AWS credentials are configured
/// This function attempts to load AWS configuration and credentials using the AWS SDK's default provider chain.
/// It checks if a credentials provider is available and if it can successfully provide credentials.
/// This is used by the health check endpoint to determine if AWS integration is properly set up.
/// In a real implementation, you might want to expand this to include more detailed checks
/// (e.g. specific credential sources, ability to call a simple AWS API, etc.) depending on your application's
/// requirements.
/// Returns:
///   bool indicating whether AWS credentials are configured and can be loaded
/// Raises:
///   None. This function is designed to be resilient and return a simple boolean status without throwing errors.
/// Any issues with loading credentials will be treated as "not configured" and reflected in the false return value,
/// allowing the health check endpoint to provide appropriate guidance to the user without crashing or throwing exceptions.
pub async fn is_aws_configured(config: &aws_config::SdkConfig) -> bool {
    // Check if credentials are available
    match config.credentials_provider() {
        Some(provider) => provider.provide_credentials().await.is_ok(),
        None => false,
    }
}

/// Process-wide cache of SDK configs keyed by "profile|region", so credential providers are reused.
pub(super) static SDK_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, aws_config::SdkConfig>>,
> = std::sync::OnceLock::new();

/// Returns a cached AWS SDK config for a profile and optional region.
/// Args:
///   profile: AWS profile name.
///   region: Optional region; when absent the default provider chain is used.
/// Returns:
///   A clone of the cached SdkConfig, loading it from the environment on first use.
pub async fn sdk_config(profile: &str, region: Option<&str>) -> aws_config::SdkConfig {
    let key = format!("{}|{}", profile, region.unwrap_or_default());
    let cache = SDK_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(cfg) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return cfg;
    }
    let cfg = aws_config::from_env()
        .profile_name(profile)
        .region(region_chain(region.map(str::to_string)))
        .load()
        .await;
    if let Ok(mut c) = cache.lock() {
        c.insert(key, cfg.clone());
    }
    cfg
}

/// Helper function to get the current time in seconds since UNIX epoch. This is used for cache timestamping and expiration checks.
/// Returns:
///   Current time in seconds as an i64. If there is an error getting the system time (e.g. system clock is before UNIX epoch), it returns 0 to avoid panicking
/// Note: This function is designed to be simple and robust, providing a consistent way to get the current time for cache management.
/// Raises:
///  None. This function handles any potential errors internally and returns a fallback value if it encounters issues
/// with system time retrieval, ensuring that it does not cause panics or crashes in the application.
pub(super) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InstanceItem, Preferences};
    use tokio::sync::RwLock;

    /// Builds a minimal AppState for unit tests.
    fn test_state() -> AppState {
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        AppState {
            prefs: Arc::new(RwLock::new(Preferences::default())),
            connections: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            instance_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            instance_refreshes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            rds_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            rds_refreshes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            ecs_cache: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            ecs_refreshes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            transfers: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            accounts: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            shutdown_tx,
        }
    }

    #[test]
    /// Ensures filtering honors state and OS parameters.
    fn filter_cached_instances_applies_state_and_os_filters() {
        let entries = vec![
            InstanceItem {
                id: "i-1".into(),
                name: "one".into(),
                typ: "t3.micro".into(),
                state: "running".into(),
                os: "Linux".into(),
                has_ssm: true,
            },
            InstanceItem {
                id: "i-2".into(),
                name: "two".into(),
                typ: "t3.micro".into(),
                state: "stopped".into(),
                os: "Windows".into(),
                has_ssm: false,
            },
        ];

        let params = InstanceListParams {
            filter_state: Some("running".into()),
            filter_os: Some("Linux".into()),
        };

        let filtered = filter_cached_instances(entries, &params);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "i-1");
    }

    #[tokio::test]
    /// Verifies the refresh tracker assigns a single leader and wakes waiters.
    async fn refresh_tracker_marks_one_leader_and_one_waiter() {
        let state = test_state();

        let first = register_instance_refresh(&state, "default|us-east-1").await;
        assert!(first.leader);

        let second = register_instance_refresh(&state, "default|us-east-1").await;
        assert!(!second.leader);

        complete_instance_refresh(&state, "default|us-east-1").await;
        assert!(state.instance_refreshes.lock().await.is_empty());
    }
}
