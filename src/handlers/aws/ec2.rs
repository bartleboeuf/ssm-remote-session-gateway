use anyhow::Result;
use aws_config::retry::RetryConfig;
use aws_config::timeout::TimeoutConfig;
use aws_sdk_ec2 as ec2;
use aws_sdk_ec2::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_ssm as ssm;
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
use std::{collections::HashSet, time::Duration};
use tokio::task::JoinSet;

use crate::models::{CachedInstances, InstanceItem, WindowsPasswordReq, ok};
use crate::state::AppState;
use axum::extract::State;
use serde::Deserialize;
use tracing::{error, info, warn};

use super::common::*;

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
pub(super) async fn fetch_managed_instance_ids(
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

            // After refresh completes, check cache again for fresh data
            if let Some(cached) = {
                let cache = state.instance_cache.lock().await;
                cache.get(&cache_key).cloned()
            } {
                let now = now_secs();
                // Verify data is fresh (just refreshed), not expired
                if now - cached.fetched_at <= ttl {
                    return Json(filter_cached_instances(cached.entries, &params)).into_response();
                }
            }
            // If cache is still expired after refresh, try again (leader may have failed)
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

        // Insert fresh data into cache with current timestamp
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

        // Notify all waiting requests that refresh is complete.
        // Timing: Cache is inserted above before notify, ensuring waiters see fresh data.
        // Brief window (ms-scale) where cache is updated but notified tasks not yet resumed,
        // but when they resume, TTL check ensures they get fresh data or retry.
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

    let created_at: Option<String> = instance
        .block_device_mappings()
        .iter()
        .find(|bdm| {
            let name = bdm.device_name().unwrap_or("");
            name.starts_with("/dev/xvda") || name == "/dev/sda1"
        })
        .and_then(|bdm| bdm.ebs())
        .and_then(|ebs| ebs.attach_time())
        .map(|t| t.to_string());

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
        "created_at": created_at,
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
pub(super) async fn resolve_pem_key(
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
pub(super) fn decrypt_password(
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
