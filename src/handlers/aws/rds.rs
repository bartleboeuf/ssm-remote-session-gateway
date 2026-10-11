use aws_config::retry::RetryConfig;
use aws_config::timeout::TimeoutConfig;
use aws_sdk_rds as rds;
use axum::{Json, http::StatusCode, response::IntoResponse};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

use crate::models::{CachedRdsInstances, RdsInstance};
use crate::state::AppState;
use axum::extract::State;
use tracing::{info, warn};

use super::common::*;

/// Tests if RDS describe_db_instances can be called with current credentials/profile
pub(super) async fn can_access_rds(config: &aws_config::SdkConfig) -> bool {
    let rds_client = rds::Client::new(config);

    match rds_client.describe_db_instances().send().await {
        Ok(_) => true,
        Err(e) => {
            warn!("RDS health check failed: {e}");
            false
        }
    }
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
    let mut entries = Vec::new();
    let mut next_token: Option<String> = None;

    loop {
        let mut req = client.describe_db_instances();
        if let Some(token) = next_token.take() {
            req = req.marker(token);
        }

        match req.send().await {
            Ok(resp) => {
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

                next_token = resp.marker().map(|m| m.to_string());
                if next_token.is_none() {
                    break;
                }
            }
            Err(e) => {
                warn!("list_rds_databases describe_db_instances failed: {e}");
                let mut refreshes = state.rds_refreshes.lock().await;
                refreshes.remove(&cache_key);
                drop(refreshes);

                return crate::models::err(StatusCode::BAD_REQUEST, format!("AWS error: {}", e))
                    .into_response();
            }
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
    if let Some(notify) = refreshes.remove(&cache_key) {
        drop(refreshes);
        notify.notify_waiters();
    } else {
        drop(refreshes);
    }

    info!("list_rds_databases success: {} databases", entries.len());
    Json(entries).into_response()
}
