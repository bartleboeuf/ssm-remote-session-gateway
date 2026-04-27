use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_config::meta::region::RegionProviderChain;
use aws_config::retry::RetryConfig;
use aws_config::timeout::TimeoutConfig;
use aws_credential_types::provider::ProvideCredentials;
use aws_sdk_ec2 as ec2;
use aws_sdk_ec2::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_ecs as ecs;
use aws_sdk_iam as iam;
use aws_sdk_rds as rds;
use aws_sdk_ssm as ssm;
use aws_sdk_sts as sts;
use aws_types::region::Region;
use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use which::which;

use crate::models::{
    CachedEcsTasks, CachedInstances, CachedRdsInstances, ConnectRequest, EcsTask, ErrorResponse,
    InstanceItem, RdsInstance, WindowsPasswordReq, ok,
};
use crate::state::AppState;
use axum::extract::State;
use serde::Deserialize;
use tracing::{error, info, warn};

/// Builds a fully configured AWS SDK loader, allowing overrides for retries and timeouts.
async fn load_aws_config(
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
fn region_chain(region: Option<String>) -> RegionProviderChain {
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
async fn save_instance_cache(
    cache: &std::collections::HashMap<String, CachedInstances>,
) -> Result<()> {
    crate::models::Preferences::ensure_cache_dir().await?;
    let path = crate::models::Preferences::cache_path();
    let data = serde_json::to_string_pretty(cache)?;
    tokio::fs::write(path, data).await?;
    Ok(())
}

struct RefreshSlot {
    notify: Arc<Notify>,
    leader: bool,
}

/// Claims a refresh slot for the given cache key, preventing concurrent AWS calls.
async fn register_instance_refresh(state: &AppState, cache_key: &str) -> RefreshSlot {
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
async fn complete_instance_refresh(state: &AppState, cache_key: &str) {
    let notify = {
        let mut refreshes = state.instance_refreshes.lock().await;
        refreshes.remove(cache_key)
    };

    if let Some(notify) = notify {
        notify.notify_waiters();
    }
}

/// Filters cached instances according to the requested state and OS filters.
fn filter_cached_instances(
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
pub async fn is_aws_configured() -> bool {
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;

    // Check if credentials are available
    match config.credentials_provider() {
        Some(provider) => provider.provide_credentials().await.is_ok(),
        None => false,
    }
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
pub async fn health() -> impl IntoResponse {
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

    Json(serde_json::json!({
        "aws_cli": which("aws").is_ok(),
        "session_manager_plugin": smp_installed,
        "session_manager_plugin_version": smp_version,
        "aws_credentials": is_aws_configured().await,
        "os": std::env::consts::OS,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// List available AWS CLI profiles by parsing ~/.aws/credentials and ~/.aws/config files.
/// Returns:
///   List of profile names (e.g. ["default", "dev", "prod", ...])
/// Raises:
///   None. If no profiles are found, it will return a list with "default" as the only profile.
/// This is because AWS SDK will fall back to "default" profile if no profiles are defined,
/// so we want to reflect that in the API response. The function will attempt to read both
/// credentials and config files to gather all profile names, and it will handle any file reading
/// errors gracefully by simply ignoring them and returning whatever profiles it could find.
/// Note: This function does not validate the profiles or check if they have valid credentials.
/// It simply lists the profile names based on the presence of profile sections in the AWS CLI
/// configuration files. In a real implementation, you might want to add additional checks to filter
/// out invalid profiles or provide more details about each profile (e.g. which credentials file it
/// comes from, whether it has valid credentials, etc.).
pub async fn list_profiles() -> impl IntoResponse {
    info!("list_profiles called");
    let mut profiles = std::collections::BTreeSet::new();
    if let Some(user_dirs) = directories::UserDirs::new() {
        let home = user_dirs.home_dir();
        for fname in [".aws/credentials", ".aws/config"] {
            let path = home.join(fname);
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                for line in content.lines() {
                    let line = line.trim();
                    if line.starts_with('[') && line.ends_with(']') {
                        let name = &line[1..line.len() - 1];
                        let clean = name.strip_prefix("profile ").unwrap_or(name).to_string();
                        profiles.insert(clean);
                    }
                }
            }
        }
    }
    if profiles.is_empty() {
        profiles.insert("default".into());
    }
    Json(profiles.into_iter().collect::<Vec<_>>())
}

#[derive(Debug, Deserialize, Default)]
pub struct RegionParams {
    pub profile: Option<String>,
}

/// List AWS regions available to the current credentials/profile.
/// Args:
///   profile: AWS CLI profile name to use for credentials and config (optional, defaults to "default" or last used)
/// Returns:
///   List of region names (e.g. ["us-east-1", "us-west-2", ...])
/// Raises:
///   ClientError: If AWS API call fails or credentials are invalid. The error message will include details from
/// the AWS SDK error to help diagnose issues (e.g. invalid credentials, permission issues, network issues).
/// Note: This function will attempt to load AWS configuration and credentials based on the provided profile,
/// falling back to defaults if not specified. It will then call EC2 DescribeRegions API to retrieve the list
/// of regions available to the credentials. If successful, it returns a sorted list of region names.
/// If it encounters any errors (e.g. invalid credentials, permission issues), it logs the error details and
/// returns a structured JSON response with an error message.
pub async fn list_regions(
    State(state): State<AppState>,
    Query(params): Query<RegionParams>,
) -> impl IntoResponse {
    info!("list_regions called");
    let prefs = state.prefs.read().await;
    let profile = params
        .profile
        .clone()
        .or_else(|| prefs.last_profile.clone())
        .unwrap_or_else(|| "default".into());

    let cfg = load_aws_config(&profile, region_chain(None), None, None).await;

    let client = ec2::Client::new(&cfg);
    match client
        .describe_regions()
        .all_regions(true)
        .filters(
            ec2::types::Filter::builder()
                .name("opt-in-status")
                .values("opt-in-not-required")
                .values("opted-in")
                .build(),
        )
        .send()
        .await
    {
        Ok(resp) => {
            let region_list: &[ec2::types::Region] = resp.regions();
            let mut regions: Vec<String> = region_list
                .iter()
                .filter_map(|r: &ec2::types::Region| r.region_name().map(|s| s.to_string()))
                .collect();

            regions.sort();
            info!("list_regions success: {} regions", regions.len());
            Json(serde_json::json!({ "ok": true, "data": regions, "error": null }))
        }
        Err(e) => {
            warn!("list_regions failed: {e}");
            println!("AWS error details: {:#?}", e);
            Json(serde_json::json!({
                "ok": false,
                "data": [],
                "error": e.to_string()
            }))
        }
    }
}

/// Validates AWS credentials by calling STS GetCallerIdentity API and returns account information.
/// Args:
///   profile: AWS CLI profile name to use for credentials and config (optional, defaults to "default" or last used)
///   region: AWS region to use for API calls (optional, defaults to last used or env/default)
/// Returns:
///   account_id: AWS account ID associated with the credentials
///   account_alias: (optional) AWS account alias if available, requires additional IAM permissions
/// Raises:
///   BadRequest: If credential validation fails or API calls return errors. The error message will include details
/// from the AWS SDK error to help diagnose issues (e.g. invalid credentials, permission issues
/// Note: This function will attempt to load AWS configuration and credentials based on the provided profile and region,
/// falling back to defaults if not specified. It will then call STS GetCallerIdentity to validate the credentials and
/// extract the account ID. If successful, it will also attempt to retrieve the account alias using IAM ListAccountAliases,
/// but this is optional and will not cause the whole operation to fail if it encounters permission issues.
/// The function returns a structured JSON response with the account information or an error message.
pub async fn connect(
    State(state): State<AppState>,
    Json(body): Json<ConnectRequest>,
) -> Result<Json<crate::models::ApiResponse<serde_json::Value>>, (StatusCode, Json<ErrorResponse>)>
{
    info!(
        "connect called profile={} region={}",
        body.profile, body.region
    );
    let cfg = load_aws_config(
        &body.profile,
        region_chain(Some(body.region.clone())),
        Some(RetryConfig::standard().with_max_attempts(3)),
        Some(
            TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .build(),
        ),
    )
    .await;

    let sts_client = sts::Client::new(&cfg);
    let identity = match sts_client.get_caller_identity().send().await {
        Ok(resp) => resp,
        Err(e) => {
            warn!("connect credential validation failed: {e}");
            return Err(crate::models::err(
                StatusCode::BAD_REQUEST,
                format!("AWS credential validation failed: {e}"),
            ));
        }
    };

    let account_id = identity.account().unwrap_or_default().to_string();

    let iam_client = iam::Client::new(&cfg);
    let account_alias = match iam_client.list_account_aliases().send().await {
        Ok(resp) => resp.account_aliases().first().cloned(),
        Err(err) => {
            warn!("connect alias lookup failed: {err}");
            None
        }
    };

    let prefs_snapshot = {
        let mut prefs = state.prefs.write().await;
        prefs.last_profile = Some(body.profile.clone());
        prefs.last_region = Some(body.region.clone());
        prefs.clone()
    };
    prefs_snapshot.save().await;

    info!("connect success account_id={}", account_id);
    Ok(ok(serde_json::json!({
        "account_id": account_id,
        "account_alias": account_alias
    })))
}

/// List EC2 instances in the current region with optional state filter.
#[derive(Debug, Deserialize, Default)]
pub struct InstanceListParams {
    pub filter_state: Option<String>,
    pub filter_os: Option<String>,
}

/// Returns the subset of instance IDs that are managed by SSM.
///
/// This batches `DescribeInstanceInformation` calls so a large region only needs a few
/// SSM requests instead of one request per EC2 instance.
async fn fetch_managed_instance_ids(
    client: &ssm::Client,
    instance_ids: &[String],
) -> HashSet<String> {
    const BATCH_SIZE: usize = 40;
    const MAX_CONCURRENT_BATCHES: usize = 6;

    let mut managed_ids = HashSet::new();
    let mut join_set = JoinSet::new();
    let mut in_flight = 0usize;

    for batch in instance_ids.chunks(BATCH_SIZE) {
        let client = client.clone();
        let batch = batch.to_vec();
        join_set.spawn(async move {
            let mut filter =
                ssm::types::InstanceInformationStringFilter::builder().key("InstanceIds");
            for instance_id in &batch {
                filter = filter.values(instance_id.clone());
            }

            let mut batch_ids = HashSet::new();
            match client
                .describe_instance_information()
                .filters(filter.build().unwrap())
                .max_results(batch.len() as i32)
                .send()
                .await
            {
                Ok(resp) => {
                    for info in resp.instance_information_list() {
                        if let Some(instance_id) = info.instance_id() {
                            batch_ids.insert(instance_id.to_string());
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "SSM batch availability check failed for {} instances: {e}",
                        batch.len()
                    );
                }
            }

            batch_ids
        });
        in_flight += 1;
        if in_flight >= MAX_CONCURRENT_BATCHES
            && let Some(Ok(batch_ids)) = join_set.join_next().await
        {
            managed_ids.extend(batch_ids);
            in_flight -= 1;
        }
    }

    while in_flight > 0 {
        match join_set.join_next().await {
            Some(Ok(batch_ids)) => managed_ids.extend(batch_ids),
            Some(Err(err)) => warn!("SSM batch task failed: {err}"),
            None => break,
        }
        in_flight -= 1;
    }

    managed_ids
}

/// List EC2 instances in the current region with optional state filter.
/// Args:
///   filter_state: Optional instance state to filter by (e.g. "running", "stopped")
/// Returns:
///   List of instances with basic information (id, name, type, state, platform, has_ssm)
/// Raises:
///   ClientError: If AWS API call fails
pub async fn list_instances(
    State(state): State<AppState>,
    Query(params): Query<InstanceListParams>,
) -> impl IntoResponse {
    info!(
        "list_instances called filter_state={:?}",
        params.filter_state
    );
    load_instance_cache_from_disk(&state).await;

    let prefs = state.prefs.read().await.clone();
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let region_provider = region_chain(prefs.last_region.clone());
    let resolved_region = region_provider
        .region()
        .await
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());
    let cache_key = format!("{}|{}", profile, resolved_region);
    let ttl = prefs.cache_ttl_secs as i64;

    if let Some(cached) = {
        let cache = state.instance_cache.lock().await;
        cache.get(&cache_key).cloned()
    } {
        let now = now_secs();
        if now - cached.fetched_at <= ttl {
            info!("list_instances cache hit for {}", cache_key);
            return Json(filter_cached_instances(cached.entries, &params)).into_response();
        } else {
            info!("list_instances cache expired for {}", cache_key);
        }
    }
    loop {
        let slot = register_instance_refresh(&state, &cache_key).await;
        if !slot.leader {
            info!(
                "list_instances waiting for in-flight refresh for {}",
                cache_key
            );
            slot.notify.notified().await;
            if let Some(cached) = {
                let cache = state.instance_cache.lock().await;
                cache.get(&cache_key).cloned()
            } {
                let now = now_secs();
                if now - cached.fetched_at <= ttl {
                    return Json(filter_cached_instances(cached.entries, &params)).into_response();
                }
            }
            continue;
        }

        info!("list_instances cache miss for {}; querying AWS", cache_key);

        let cfg = load_aws_config(
            &profile,
            region_chain(prefs.last_region.clone()),
            Some(RetryConfig::standard().with_max_attempts(3)),
            Some(
                TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(10))
                    .read_timeout(Duration::from_secs(30))
                    .build(),
            ),
        )
        .await;

        let ec2_client = ec2::Client::new(&cfg);
        let ssm_client = ssm::Client::new(&cfg);

        let mut instance_meta: Vec<(String, String, String, String, String)> = Vec::new();
        let mut next_token: Option<String> = None;
        loop {
            let mut req = ec2_client.describe_instances();
            if let Some(token) = next_token.take() {
                req = req.next_token(token);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("list_instances describe_instances failed: {e}");
                    complete_instance_refresh(&state, &cache_key).await;
                    return crate::models::err(
                        StatusCode::BAD_REQUEST,
                        format!("AWS error: {}", e.message().unwrap_or_default()),
                    )
                    .into_response();
                }
            };

            for inst in resp.reservations().iter().flat_map(|r| r.instances()) {
                let iid = inst.instance_id().unwrap_or("").to_string();
                let name = inst
                    .tags()
                    .iter()
                    .find(|t| t.key() == Some("Name"))
                    .and_then(|t| t.value())
                    .unwrap_or(iid.as_str())
                    .to_string();
                let platform = inst.platform_details().unwrap_or("Linux");
                let state = inst
                    .state()
                    .and_then(|s| s.name())
                    .map(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let typ = inst
                    .instance_type()
                    .map(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let os = if platform.contains("Windows") {
                    "Windows".into()
                } else {
                    "Linux".into()
                };

                instance_meta.push((iid, name, typ, state, os));
            }

            next_token = resp.next_token().map(|t| t.to_string());
            if next_token.is_none() {
                break;
            }
        }

        let instance_ids: Vec<String> = instance_meta
            .iter()
            .map(|(id, _, _, _, _)| id.clone())
            .collect();
        let managed_instance_ids = fetch_managed_instance_ids(&ssm_client, &instance_ids).await;

        let instances: Vec<InstanceItem> = instance_meta
            .into_iter()
            .map(|(id, name, typ, state, os)| InstanceItem {
                has_ssm: managed_instance_ids.contains(&id),
                id,
                name,
                typ,
                state,
                os,
            })
            .collect();

        let cache_snapshot = {
            let mut cache = state.instance_cache.lock().await;
            cache.insert(
                cache_key.clone(),
                CachedInstances {
                    fetched_at: now_secs(),
                    entries: instances.clone(),
                },
            );
            cache.clone()
        };

        if let Err(e) = save_instance_cache(&cache_snapshot).await {
            warn!("Failed to persist instance cache: {e}");
        }

        complete_instance_refresh(&state, &cache_key).await;
        return Json(filter_cached_instances(instances, &params)).into_response();
    }
}

/// Get detailed information about a specific EC2 instance.
///
/// Args:
///    instance_id: EC2 instance ID
/// Returns:
///    Dict with instance details
/// Raises:
///    ClientError: If instance not found or API call fails
/// Note: In offline mode, this will return a 404 error since we cannot fetch real instance details.
/// In a real implementation, this function would use the AWS SDK to call describe_instances and
/// extract details like IAM role, security groups, etc.
/// For the purpose of this example, we will return a 404 error to indicate that instance details are not available in offline mode.
///
pub async fn instance_details(
    Path(instance_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    info!("instance_details called id={instance_id}");

    let prefs = state.prefs.read().await.clone();
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let cfg = load_aws_config(
        &profile,
        region_chain(prefs.last_region.clone()),
        Some(RetryConfig::standard().with_max_attempts(3)),
        Some(
            TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .build(),
        ),
    )
    .await;

    let client = ec2::Client::new(&cfg);

    let resp = match client
        .describe_instances()
        .instance_ids(instance_id.clone())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if let SdkError::ServiceError(service_err) = &e
                && service_err.err().code() == Some("InvalidInstanceID.NotFound")
            {
                warn!("instance {} not found", instance_id);
                return crate::models::err(
                    StatusCode::NOT_FOUND,
                    format!("Instance {instance_id} not found"),
                )
                .into_response();
            }
            warn!("describe_instances failed: {e}");
            return crate::models::err(StatusCode::BAD_REQUEST, format!("AWS error: {e}"))
                .into_response();
        }
    };

    let instance = match resp
        .reservations()
        .iter()
        .flat_map(|r| r.instances())
        .find(|i| i.instance_id().map(|id| id == instance_id).unwrap_or(false))
    {
        Some(i) => i,
        None => {
            return crate::models::err(
                StatusCode::NOT_FOUND,
                format!("Instance {instance_id} not found"),
            )
            .into_response();
        }
    };

    let name = instance
        .tags()
        .iter()
        .find(|t| t.key() == Some("Name"))
        .and_then(|t| t.value())
        .unwrap_or(instance_id.as_str())
        .to_string();

    let iam_role = instance
        .iam_instance_profile()
        .and_then(|p| p.arn())
        .map(|arn| arn.split('/').next_back().unwrap_or(arn).to_string())
        .unwrap_or_else(|| "N/A".into());

    let security_groups = instance
        .security_groups()
        .iter()
        .map(|sg| {
            let id = sg.group_id().unwrap_or("");
            let name = sg.group_name().unwrap_or("");
            if !name.is_empty() {
                format!("{name} ({id})")
            } else {
                id.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");

    Json(serde_json::json!({
        "id": instance_id,
        "name": name,
        "type": instance
            .instance_type()
            .map(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "state": instance
            .state()
            .and_then(|s| s.name())
            .map(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        "platform": instance.platform_details().unwrap_or("").to_string(),
        "private_ip": instance
            .private_ip_address()
            .unwrap_or("N/A")
            .to_string(),
        "public_ip": instance
            .public_ip_address()
            .unwrap_or("N/A")
            .to_string(),
        "vpc_id": instance.vpc_id().unwrap_or("N/A").to_string(),
        "subnet_id": instance.subnet_id().unwrap_or("N/A").to_string(),
        "iam_role": iam_role,
        "ami_id": instance.image_id().unwrap_or("N/A").to_string(),
        "key_name": instance.key_name().unwrap_or("N/A").to_string(),
        "security_groups": if security_groups.is_empty() { "N/A".into() } else { security_groups },
    }))
    .into_response()
}

/// Retrieve and decrypt the Windows administrator password for a given EC2 instance.
/// Args:
///  instance_id: ID of the EC2 instance to retrieve the password for
///  pem_key: (optional) PEM-encoded RSA private key content to use for decryption. If not provided,
/// the function will attempt to locate the key file based on the key_name and configured ssh_key_folder in preferences.
///  key_name: (optional) Name of the key pair associated with the instance.
/// This is used to locate the PEM key file if pem_key is not provided directly in the request.
/// Returns:
///  Decrypted administrator password as a string if successful, or an error response if the password is not available,
/// decryption fails, or if there are issues with the provided key information.
/// Note: This function first retrieves the encrypted password data from AWS using the GetPasswordData API.
/// If the password data is not yet available (e.g. instance is still initializing), it returns an error indicating that
/// the password is not ready. It then resolves the PEM key either from the request body or by looking up the key file
/// based on the provided key name and configured SSH key folders. Finally, it attempts to decrypt the password using
/// the RSA private key. If any step fails (e.g. invalid instance ID, password not available, key resolution failure, decryption error),
/// it returns a structured error response with details about the failure.
pub async fn windows_password(
    Path(instance_id): Path<String>,
    State(state): State<AppState>,
    Json(body): Json<WindowsPasswordReq>,
) -> Result<
    Json<crate::models::ApiResponse<serde_json::Value>>,
    (StatusCode, Json<crate::models::ErrorResponse>),
> {
    let prefs = state.prefs.read().await.clone();

    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let cfg = load_aws_config(
        &profile,
        region_chain(prefs.last_region.clone()),
        Some(RetryConfig::standard()),
        Some(
            TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(20))
                .build(),
        ),
    )
    .await;

    let ec2_client = ec2::Client::new(&cfg);
    let resp = match ec2_client
        .get_password_data()
        .instance_id(&instance_id)
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            let msg = match err.as_service_error() {
                Some(se) => se
                    .message()
                    .unwrap_or("Failed to get password data")
                    .to_string(),
                None => err.to_string(),
            };
            return Err(crate::models::err(StatusCode::BAD_REQUEST, msg));
        }
    };

    let encrypted = resp.password_data().unwrap_or("");
    if encrypted.is_empty() {
        return Err(crate::models::err(
            StatusCode::BAD_REQUEST,
            "Password is not yet available. Try again after the instance finishes initializing.",
        ));
    }

    let pem = match resolve_pem_key(&body, &prefs).await {
        Ok(k) => k,
        Err(e) => return Err(e),
    };

    let password = match decrypt_password(&pem, encrypted) {
        Ok(p) => p,
        Err(e) => return Err(e),
    };

    Ok(ok(serde_json::json!({ "password": password })))
}

/// Returns the current application version for the frontend and API consumers.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({"version": env!("CARGO_PKG_VERSION")}))
}

/// Resolves the PEM key content either directly from the request body or by looking up the key file based
/// on the provided key name and configured SSH key folders.
/// Args:
///   body: WindowsPasswordReq containing either pem_key or key_name
///   prefs: Preferences containing ssh_key_folder configuration
/// Returns:
///   PEM key content as a string if found, or an error response tuple if the key cannot be resolved.
///   The function first checks if the PEM key is provided directly in the request body. If not, it
///   attempts to locate the key file based on the key_name and the configured ssh_key_folder(s). It supports
///   multiple folders separated by commas or newlines, and it tries several common filename patterns based on
///   the key_name. If the key file is found and read successfully, it returns the content. If it cannot find
///   the key or read it, it returns a structured error response indicating the issue and suggesting how to
///   configure the key properly.
/// Note: This function is designed to be flexible in how it locates the PEM key, allowing users to either
/// provide the key directly or specify a key name that can be resolved against configured folders. It also
/// provides detailed error messages to help users troubleshoot issues with key configuration. In a real
/// implementation, you might want to add additional security checks (e.g. ensuring the key file has appropriate
/// permissions) or support for different key formats if needed.
async fn resolve_pem_key(
    body: &WindowsPasswordReq,
    prefs: &crate::models::Preferences,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    if let Some(pem) = body.pem_key.clone() {
        return Ok(pem);
    }

    let key_name = body.key_name.clone().ok_or_else(|| {
        crate::models::err(
            StatusCode::BAD_REQUEST,
            "pem_key or key_name must be provided to decrypt the password",
        )
    })?;

    let ssh_key_folder = prefs.ssh_key_folder.clone().ok_or_else(|| {
        crate::models::err(
            StatusCode::BAD_REQUEST,
            "ssh_key_folder preference is not set; cannot locate key file",
        )
    })?;

    let folders = ssh_key_folder
        .split([',', '\n'])
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else if trimmed.starts_with('~') {
                directories::UserDirs::new()
                    .map(|u| u.home_dir().join(trimmed.trim_start_matches('~')))
            } else {
                Some(std::path::PathBuf::from(trimmed))
            }
        })
        .collect::<Vec<_>>();

    let candidates = [
        key_name.clone(),
        format!("{key_name}.pem"),
        format!("{key_name}.key"),
        format!("id_rsa_{key_name}"),
        format!("{key_name}_private.pem"),
    ];

    for folder in folders {
        for cand in &candidates {
            let path = folder.join(cand);
            if let Ok(data) = tokio::fs::read_to_string(&path).await {
                return Ok(data);
            }
        }
    }

    Err(crate::models::err(
        StatusCode::BAD_REQUEST,
        "Could not locate PEM key. Provide pem_key directly or configure ssh_key_folder with the key file.",
    ))
}

/// Decrypts the given base64-encoded encrypted password using the provided PEM-encoded RSA private key.
/// Args:
///   pem: PEM-encoded RSA private key as a string
///   encrypted_b64: Base64-encoded encrypted password string from AWS GetPasswordData API
/// Returns:
///   Decrypted password as a string if successful, or an error response tuple if decryption fails
/// (e.g. invalid key, invalid encrypted data, decryption error).
/// Note: This function attempts to parse the provided PEM key using both PKCS1 and PKCS8 formats to maximize
/// compatibility with different key types. It also provides detailed error messages for various failure scenarios
/// to help diagnose issues
/// with key parsing and decryption. The decryption is performed using RSA PKCS1v15 padding, which is the standard
/// for AWS Windows password encryption.
/// In a real implementation, you might want to add additional checks (e.g. key size, allowed formats) or
/// support for other encryption schemes if AWS introduces changes in the future.
/// The function is designed to be robust and provide clear error messages for common issues like invalid PEM format,
/// invalid base64 data, and decryption failures, which can help users troubleshoot problems with their key configuration.
/// Raises:
///   BadRequest: If the PEM key cannot be parsed, if the encrypted data is not valid base64, or if decryption fails.
/// The error message will include details about the failure to help diagnose issues with key configuration or data formatting.
fn decrypt_password(
    pem: &str,
    encrypted_b64: &str,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    let key = RsaPrivateKey::from_pkcs1_pem(pem).or_else(|_| RsaPrivateKey::from_pkcs8_pem(pem));

    let key = match key {
        Ok(k) => k,
        Err(e) => {
            error!("Failed to parse PEM key: {e}");
            return Err(crate::models::err(
                StatusCode::BAD_REQUEST,
                format!("Failed to parse PEM key: {e}"),
            ));
        }
    };

    let cipher = match BASE64.decode(encrypted_b64.as_bytes()) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to decode password data: {e}");
            return Err(crate::models::err(
                StatusCode::BAD_REQUEST,
                "Invalid encrypted password data",
            ));
        }
    };

    match key.decrypt(Pkcs1v15Encrypt, &cipher) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|e| {
            error!("Decrypted password is not valid UTF-8: {e}");
            crate::models::err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Decrypted password is not valid UTF-8",
            )
        }),
        Err(e) => {
            error!("RSA decryption failed: {e}");
            Err(crate::models::err(
                StatusCode::BAD_REQUEST,
                "Failed to decrypt password with provided key",
            ))
        }
    }
}

/// Helper function to get the current time in seconds since UNIX epoch. This is used for cache timestamping and expiration checks.
/// Returns:
///   Current time in seconds as an i64. If there is an error getting the system time (e.g. system clock is before UNIX epoch), it returns 0 to avoid panicking
/// Note: This function is designed to be simple and robust, providing a consistent way to get the current time for cache management.
/// Raises:
///  None. This function handles any potential errors internally and returns a fallback value if it encounters issues
/// with system time retrieval, ensuring that it does not cause panics or crashes in the application.
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// List RDS database instances in the current region with caching and coordination.
/// Uses the same cache pattern as EC2 instances to prevent duplicate AWS API calls.
pub async fn list_rds_databases(State(state): State<AppState>) -> impl IntoResponse {
    info!("list_rds_databases called");

    let prefs = state.prefs.read().await.clone();
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let region_provider = region_chain(prefs.last_region.clone());
    let resolved_region = region_provider
        .region()
        .await
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());
    let cache_key = format!("{}|{}", profile, resolved_region);
    let ttl = prefs.cache_ttl_secs as i64;

    // Check cache first
    if let Some(cached) = {
        let cache = state.rds_cache.lock().await;
        cache.get(&cache_key).cloned()
    } {
        let now = now_secs();
        if now - cached.fetched_at <= ttl {
            info!("list_rds_databases cache hit for {}", cache_key);
            return Json(cached.entries).into_response();
        }
    }

    // Register refresh slot
    let slot = {
        let mut refreshes = state.rds_refreshes.lock().await;
        if let Some(existing) = refreshes.get(&cache_key) {
            (false, existing.clone())
        } else {
            let notify = Arc::new(Notify::new());
            refreshes.insert(cache_key.clone(), notify.clone());
            (true, notify)
        }
    };

    if !slot.0 {
        // Not leader, wait for refresh
        slot.1.notified().await;
        if let Some(cached) = {
            let cache = state.rds_cache.lock().await;
            cache.get(&cache_key).cloned()
        } {
            let now = now_secs();
            if now - cached.fetched_at <= ttl {
                return Json(cached.entries).into_response();
            }
        }
    }

    // Leader: fetch from AWS
    info!("list_rds_databases fetching from AWS for {}", cache_key);

    let cfg = load_aws_config(
        &profile,
        region_chain(prefs.last_region.clone()),
        Some(RetryConfig::standard().with_max_attempts(3)),
        Some(
            TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .build(),
        ),
    )
    .await;

    let client = rds::Client::new(&cfg);
    match client.describe_db_instances().send().await {
        Ok(resp) => {
            let mut entries = Vec::new();
            for db in resp.db_instances() {
                if let Some(db_id) = db.db_instance_identifier() {
                    let endpoint = db
                        .endpoint()
                        .and_then(|e| e.address())
                        .unwrap_or("")
                        .to_string();
                    let port = db.endpoint().and_then(|e| e.port()).unwrap_or(3306) as u16;
                    let engine = db.engine().unwrap_or("unknown").to_string();
                    let status = db.db_instance_status().unwrap_or("unknown").to_string();

                    entries.push(RdsInstance {
                        db_instance_identifier: db_id.to_string(),
                        endpoint,
                        port,
                        engine,
                        status,
                    });
                }
            }

            let cached = CachedRdsInstances {
                fetched_at: now_secs(),
                entries: entries.clone(),
            };

            let mut cache = state.rds_cache.lock().await;
            cache.insert(cache_key.clone(), cached);
            drop(cache);

            let mut refreshes = state.rds_refreshes.lock().await;
            refreshes.remove(&cache_key);
            drop(refreshes);

            info!("list_rds_databases success: {} databases", entries.len());
            Json(entries).into_response()
        }
        Err(e) => {
            warn!("list_rds_databases failed: {e}");
            let mut refreshes = state.rds_refreshes.lock().await;
            refreshes.remove(&cache_key);
            drop(refreshes);

            crate::models::err(StatusCode::BAD_REQUEST, format!("AWS error: {}", e)).into_response()
        }
    }
}

/// List ECS tasks in the current region with caching and coordination.
/// Uses the same cache pattern as EC2 instances to prevent duplicate AWS API calls.
pub async fn list_ecs_tasks(State(state): State<AppState>) -> impl IntoResponse {
    info!("list_ecs_tasks called");

    let prefs = state.prefs.read().await.clone();
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let region_provider = region_chain(prefs.last_region.clone());
    let resolved_region = region_provider
        .region()
        .await
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());
    let cache_key = format!("{}|{}", profile, resolved_region);
    let ttl = prefs.cache_ttl_secs as i64;

    // Check cache first
    if let Some(cached) = {
        let cache = state.ecs_cache.lock().await;
        cache.get(&cache_key).cloned()
    } {
        let now = now_secs();
        if now - cached.fetched_at <= ttl {
            info!("list_ecs_tasks cache hit for {}", cache_key);
            return Json(cached.entries).into_response();
        }
    }

    // Register refresh slot
    let slot = {
        let mut refreshes = state.ecs_refreshes.lock().await;
        if let Some(existing) = refreshes.get(&cache_key) {
            (false, existing.clone())
        } else {
            let notify = Arc::new(Notify::new());
            refreshes.insert(cache_key.clone(), notify.clone());
            (true, notify)
        }
    };

    if !slot.0 {
        // Not leader, wait for refresh
        slot.1.notified().await;
        if let Some(cached) = {
            let cache = state.ecs_cache.lock().await;
            cache.get(&cache_key).cloned()
        } {
            let now = now_secs();
            if now - cached.fetched_at <= ttl {
                return Json(cached.entries).into_response();
            }
        }
    }

    // Leader: fetch from AWS
    info!("list_ecs_tasks fetching from AWS for {}", cache_key);

    let cfg = load_aws_config(
        &profile,
        region_chain(prefs.last_region.clone()),
        Some(RetryConfig::standard().with_max_attempts(3)),
        Some(
            TimeoutConfig::builder()
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .build(),
        ),
    )
    .await;

    let ecs_client = ecs::Client::new(&cfg);
    let mut entries = Vec::new();

    // List clusters first
    match ecs_client.list_clusters().send().await {
        Ok(clusters_resp) => {
            for cluster_arn in clusters_resp.cluster_arns() {
                let cluster_name = cluster_arn
                    .split('/')
                    .next_back()
                    .unwrap_or(cluster_arn)
                    .to_string();

                // List tasks in this cluster
                if let Ok(tasks_resp) = ecs_client
                    .list_tasks()
                    .cluster(cluster_arn)
                    .desired_status(ecs::types::DesiredStatus::Running)
                    .send()
                    .await
                {
                    for task_arn in tasks_resp.task_arns() {
                        // Describe task to get details
                        if let Ok(describe_resp) = ecs_client
                            .describe_tasks()
                            .cluster(cluster_arn)
                            .tasks(task_arn)
                            .send()
                            .await
                        {
                            for task in describe_resp.tasks() {
                                if let Some(task_def) = task.task_definition_arn() {
                                    let task_def_name = task_def
                                        .split('/')
                                        .nth_back(1)
                                        .unwrap_or("unknown")
                                        .to_string();

                                    let container_name = task
                                        .containers()
                                        .first()
                                        .and_then(|c| c.name())
                                        .unwrap_or("unknown")
                                        .to_string();

                                    let launch_type = task
                                        .launch_type()
                                        .map(|lt| format!("{:?}", lt))
                                        .unwrap_or_else(|| "EC2".to_string());

                                    let status = task
                                        .last_status()
                                        .map(|s| format!("{:?}", s))
                                        .unwrap_or_else(|| "UNKNOWN".to_string());

                                    entries.push(EcsTask {
                                        task_arn: task_arn.to_string(),
                                        task_definition: task_def_name,
                                        container_name,
                                        launch_type,
                                        status,
                                        cluster_name: cluster_name.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!("list_ecs_tasks failed to list clusters: {e}");
            let mut refreshes = state.ecs_refreshes.lock().await;
            refreshes.remove(&cache_key);
            drop(refreshes);

            return crate::models::err(StatusCode::BAD_REQUEST, format!("AWS error: {}", e))
                .into_response();
        }
    }

    let cached = CachedEcsTasks {
        fetched_at: now_secs(),
        entries: entries.clone(),
    };

    let mut cache = state.ecs_cache.lock().await;
    cache.insert(cache_key.clone(), cached);
    drop(cache);

    let mut refreshes = state.ecs_refreshes.lock().await;
    refreshes.remove(&cache_key);
    drop(refreshes);

    info!("list_ecs_tasks success: {} tasks", entries.len());
    Json(entries).into_response()
}
