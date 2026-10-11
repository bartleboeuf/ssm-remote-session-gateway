use aws_config::retry::RetryConfig;
use aws_config::timeout::TimeoutConfig;
use aws_sdk_ecs as ecs;
use axum::{Json, http::StatusCode, response::IntoResponse};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

use crate::models::{CachedEcsTasks, EcsTask};
use crate::state::AppState;
use axum::extract::State;
use tracing::{info, warn};

use super::common::*;

/// Tests if ECS list_clusters can be called with current credentials/profile
pub(super) async fn can_access_ecs(config: &aws_config::SdkConfig) -> bool {
    let ecs_client = ecs::Client::new(config);

    match ecs_client.list_clusters().send().await {
        Ok(_) => true,
        Err(e) => {
            warn!("ECS health check failed: {e}");
            false
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

    // List clusters first with pagination
    let mut cluster_token: Option<String> = None;
    loop {
        let mut cluster_req = ecs_client.list_clusters();
        if let Some(token) = cluster_token.take() {
            cluster_req = cluster_req.next_token(token);
        }

        let clusters_resp = match cluster_req.send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!("list_ecs_tasks failed to list clusters: {e}");
                let mut refreshes = state.ecs_refreshes.lock().await;
                refreshes.remove(&cache_key);
                drop(refreshes);
                return crate::models::err(StatusCode::BAD_REQUEST, format!("AWS error: {}", e))
                    .into_response();
            }
        };

        for cluster_arn in clusters_resp.cluster_arns() {
            let cluster_name = cluster_arn
                .split('/')
                .next_back()
                .unwrap_or(cluster_arn)
                .to_string();

            // List tasks in this cluster with pagination
            let mut task_token: Option<String> = None;
            loop {
                let mut tasks_req = ecs_client
                    .list_tasks()
                    .cluster(cluster_arn)
                    .desired_status(ecs::types::DesiredStatus::Running);
                if let Some(token) = task_token.take() {
                    tasks_req = tasks_req.next_token(token);
                }

                let tasks_resp = match tasks_req.send().await {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!(
                            "list_ecs_tasks failed to list tasks in cluster {}: {e}",
                            cluster_arn
                        );
                        break;
                    }
                };

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

                task_token = tasks_resp.next_token().map(|t| t.to_string());
                if task_token.is_none() {
                    break;
                }
            }
        }

        cluster_token = clusters_resp.next_token().map(|t| t.to_string());
        if cluster_token.is_none() {
            break;
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
    if let Some(notify) = refreshes.remove(&cache_key) {
        drop(refreshes);
        notify.notify_waiters();
    } else {
        drop(refreshes);
    }

    info!("list_ecs_tasks success: {} tasks", entries.len());
    Json(entries).into_response()
}
