use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::models::{AwsPref, Logging, PortRange, PrefsInput, err, ok};
use crate::state::AppState;

/// Returns the currently stored application preferences as JSON.
pub async fn preferences_get(State(state): State<AppState>) -> impl IntoResponse {
    let prefs = state.prefs.read().await;
    tracing::info!("preferences_get: returning preferences");
    let mut aws_obj = serde_json::Map::new();
    if let Some(p) = &prefs.last_profile {
        aws_obj.insert("profile".into(), serde_json::json!(p));
    }
    if let Some(r) = &prefs.last_region {
        aws_obj.insert("region".into(), serde_json::json!(r));
    }
    let preferred = serde_json::json!({
        "profile": prefs.preferred_profile,
        "region": prefs.preferred_region,
    });

    Json(serde_json::json!({
        "port_range": {
            "start": prefs.port_range_start,
            "end": prefs.port_range_end,
        },
        "logging": {
            "level": prefs.logging_level,
            "format": prefs.logging_format,
        },
        "aws": if aws_obj.is_empty() { serde_json::Value::Null } else { serde_json::Value::Object(aws_obj) },
        "preferred_aws": preferred,
        "ssh_key_folder": prefs.ssh_key_folder,
        "ssh_options": prefs.ssh_options,
        "cache_ttl_secs": prefs.cache_ttl_secs,
        "linux_username": prefs.linux_username,
        "windows_username": prefs.windows_username,
        "dark_mode": prefs.dark_mode,
        "strict_host_key_checking": prefs.strict_host_key_checking,
    }))
}

/// Atomically updates preferences with exclusive write lock held through persistence.
/// Returns the updated preferences snapshot.
/// All updates are applied within a single lock hold, then persisted atomically.
async fn atomic_update_preferences<F>(state: &AppState, update_fn: F) -> crate::models::Preferences
where
    F: FnOnce(&mut crate::models::Preferences),
{
    let saved_prefs = {
        let mut prefs = state.prefs.write().await;
        update_fn(&mut prefs);
        prefs.clone()
    };
    saved_prefs.save().await;
    saved_prefs
}

/// Validates and persists updated preferences, then returns the saved snapshot.
/// Guarantees atomicity: preferences are read, validated, updated, and persisted
/// without releasing the write lock, preventing lost updates from concurrent requests.
pub async fn preferences_set(
    State(state): State<AppState>,
    Json(body): Json<PrefsInput>,
) -> Result<
    Json<crate::models::ApiResponse<crate::models::Preferences>>,
    (StatusCode, Json<crate::models::ErrorResponse>),
> {
    // Validate before acquiring lock to fail fast
    if let Some(PortRange { start, end }) = body.port_range {
        if start < 1024 {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Start port must be at least 1024",
            ));
        }
        if end < 1024 {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "End port must be at least 1024",
            ));
        }
        if start >= end {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "Start port must be less than end port",
            ));
        }
    }

    let saved_prefs = atomic_update_preferences(&state, |prefs| {
        if let Some(PortRange { start, end }) = body.port_range {
            prefs.port_range_start = start;
            prefs.port_range_end = end;
        }
        if let Some(Logging { level, format }) = body.logging {
            prefs.logging_level = level.to_uppercase();
            if let Some(f) = format {
                prefs.logging_format = f;
            }
        }
        if let Some(AwsPref { profile, region }) = body.aws {
            prefs.last_profile = profile;
            prefs.last_region = region;
        }
        if let Some(AwsPref { profile, region }) = body.preferred_aws {
            prefs.preferred_profile = profile.filter(|p| !p.is_empty());
            prefs.preferred_region = region.filter(|r| !r.is_empty());
        }
        if let Some(s) = body.ssh_key_folder {
            prefs.ssh_key_folder = Some(s);
        }
        if let Some(o) = body.ssh_options {
            prefs.ssh_options = o;
        }
        if let Some(ttl) = body.cache_ttl_secs {
            prefs.cache_ttl_secs = ttl.max(60); // minimum 60s to avoid hammering
        }
        if let Some(u) = body.linux_username {
            let u = u.trim().to_string();
            if !u.is_empty() {
                prefs.linux_username = u;
            }
        }
        if let Some(u) = body.windows_username {
            let u = u.trim().to_string();
            if !u.is_empty() {
                prefs.windows_username = u;
            }
        }
        if let Some(d) = body.dark_mode {
            prefs.dark_mode = d;
        }
        if let Some(s) = body.strict_host_key_checking {
            prefs.strict_host_key_checking = s;
        }
    })
    .await;

    tracing::info!("preferences_set: saved preferences");
    Ok(ok(saved_prefs))
}

/// Returns the last AWS profile and region used by the application.
pub async fn last_connection(State(state): State<AppState>) -> impl IntoResponse {
    let prefs = state.prefs.read().await;
    Json(serde_json::json!({
        "profile": prefs.last_profile.clone().unwrap_or_default(),
        "region": prefs.last_region.clone().unwrap_or_default(),
        "preferred_aws": {
            "profile": prefs.preferred_profile,
            "region": prefs.preferred_region,
        },
    }))
}
