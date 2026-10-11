use anyhow::Result;
use aws_config::retry::RetryConfig;
use aws_config::timeout::TimeoutConfig;
use aws_sdk_ec2 as ec2;
use aws_sdk_iam as iam;
use aws_sdk_sts as sts;
use axum::{Json, extract::Query, http::StatusCode, response::IntoResponse};
use std::time::Duration;

use crate::models::{ConnectRequest, ErrorResponse, ok};
use crate::state::AppState;
use axum::extract::State;
use serde::Deserialize;
use tracing::{info, warn};

use super::common::*;

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
    let prefs = state.prefs.read().await.clone();
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

    // Update profile and region atomically (held within write lock through save)
    let prefs_snapshot = {
        let mut prefs = state.prefs.write().await;
        prefs.last_profile = Some(body.profile.clone());
        prefs.last_region = Some(body.region.clone());
        prefs.clone()
    };
    prefs_snapshot.save().await;

    // Record the account for this profile so every connection on it can show the account name
    state.accounts.lock().await.insert(
        body.profile.clone(),
        crate::models::AccountInfo {
            account_id: account_id.clone(),
            account_alias: account_alias.clone(),
        },
    );

    info!("connect success account_id={}", account_id);
    Ok(ok(serde_json::json!({
        "account_id": account_id,
        "account_alias": account_alias
    })))
}
