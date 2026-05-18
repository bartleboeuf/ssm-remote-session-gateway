//! SCP file transfer handler — upload/download files through an SSM port-forward tunnel on port 22.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::{Arc, atomic::Ordering};
use tokio::io::AsyncReadExt;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::handlers::connections::{find_free_port, find_key_path, start_ssm_port_forward};
use crate::models::{
    ConnectionInfo, ManagedTransfer, TransferDirection, TransferProgress, TransferRequest,
    TransferStatus,
};
use crate::state::AppState;

/// GET /api/check-scp
/// Returns `{"available": true}` when `scp` is on PATH, otherwise `{"available": false, "hint": "..."}`.
pub async fn check_scp() -> impl IntoResponse {
    let available = which::which("scp").is_ok();
    let hint: Option<&str> = if !available {
        Some(
            "Install OpenSSH client: on Windows run `winget install Microsoft.OpenSSH.Beta`, \
             on Debian/Ubuntu run `sudo apt install openssh-client`",
        )
    } else {
        None
    };
    Json(serde_json::json!({ "available": available, "hint": hint }))
}

/// POST /api/transfer/{instance_id}
/// Starts an SCP transfer through an SSM tunnel. Returns `{"id": "<uuid>"}` immediately.
pub async fn start_transfer(
    Path(instance_id): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<TransferRequest>,
) -> impl IntoResponse {
    let prefs = state.prefs.read().await.clone();

    // Resolve username
    let username = req
        .username
        .clone()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| prefs.linux_username.clone());

    // Resolve SSH key path
    let key_path: String = if let Some(kp) = &req.key_path {
        if !kp.is_empty() {
            kp.clone()
        } else {
            String::new()
        }
    } else {
        // Auto-detect: first look up the key name from EC2, then find the file
        let key_name = {
            let profile = prefs
                .last_profile
                .clone()
                .unwrap_or_else(|| "default".into());
            use aws_config::meta::region::RegionProviderChain;
            use aws_sdk_ec2 as ec2;
            use aws_types::region::Region;

            let region_provider = prefs
                .last_region
                .clone()
                .map(Region::new)
                .map(RegionProviderChain::first_try)
                .unwrap_or_else(RegionProviderChain::default_provider);

            let cfg = aws_config::from_env()
                .profile_name(&profile)
                .region(region_provider)
                .load()
                .await;

            let client = ec2::Client::new(&cfg);
            match client
                .describe_instances()
                .instance_ids(&instance_id)
                .send()
                .await
            {
                Ok(resp) => resp
                    .reservations()
                    .iter()
                    .flat_map(|r| r.instances())
                    .find_map(|i| i.key_name().map(|s| s.to_string())),
                Err(e) => {
                    warn!("transfer: could not look up key name for {instance_id}: {e}");
                    None
                }
            }
        };

        key_name
            .as_deref()
            .and_then(|k| find_key_path(k, prefs.ssh_key_folder.as_deref()))
            .unwrap_or_default()
    };

    // Allocate a free local port for the SSH tunnel
    let local_port = match find_free_port(prefs.port_range_start, prefs.port_range_end) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "No free port available in configured range",
            )
                .into_response();
        }
    };

    // Build the connection info for the SSM tunnel (remote port 22)
    let tunnel_info = ConnectionInfo {
        id: Uuid::new_v4(),
        profile: prefs
            .last_profile
            .clone()
            .unwrap_or_else(|| "default".into()),
        region: prefs
            .last_region
            .clone()
            .unwrap_or_else(|| "us-east-1".into()),
        instance_id: instance_id.clone(),
        local_port,
        remote_port: 22,
        remote_host: None,
        command: String::new(),
        key_name: None,
        key_path: None,
    };

    // Start the SSM tunnel
    let tunnel_child = match start_ssm_port_forward(&tunnel_info, &prefs).await {
        Ok(c) => c,
        Err((status, Json(err_body))) => {
            return (status, Json(serde_json::json!({ "error": err_body.error }))).into_response();
        }
    };

    let transfer_id = Uuid::new_v4();
    let progress = Arc::new(tokio::sync::Mutex::new(TransferProgress {
        id: transfer_id.to_string(),
        status: TransferStatus::Connecting,
        percent: 0,
        speed: String::new(),
        eta: String::new(),
        error: None,
    }));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let managed = ManagedTransfer {
        id: transfer_id,
        progress: progress.clone(),
        scp_child: None,
        tunnel_child: Some(tunnel_child),
        cancel: cancel.clone(),
    };

    {
        let mut transfers = state.transfers.lock().await;
        transfers.insert(transfer_id.to_string(), managed);
    }

    // Spawn background task: wait for tunnel ready, then run scp
    let state_clone = state.clone();
    let id_str = transfer_id.to_string();
    let progress_clone = progress.clone();
    let cancel_clone = cancel.clone();
    let req_direction = req.direction.clone();
    let req_local_path = req.local_path.clone();
    let req_remote_path = req.remote_path.clone();

    tokio::spawn(async move {
        // Probe the tunnel port (up to 15 s)
        let ready = probe_port(local_port, 15).await;
        if !ready || cancel_clone.load(Ordering::Relaxed) {
            let mut p = progress_clone.lock().await;
            p.status = TransferStatus::Failed;
            p.error = Some("SSH tunnel did not become reachable within 15 seconds".into());
            info!("transfer {id_str}: tunnel timeout");
            // kill the tunnel
            let mut transfers = state_clone.transfers.lock().await;
            if let Some(t) = transfers.get_mut(&id_str)
                && let Some(child) = t.tunnel_child.as_mut()
            {
                let _ = child.kill().await;
            }
            return;
        }

        info!("transfer {id_str}: tunnel ready on port {local_port}");
        {
            let mut p = progress_clone.lock().await;
            p.status = TransferStatus::Transferring;
        }

        // Build scp command
        let mut cmd = tokio::process::Command::new("scp");
        cmd.args(["-P", &local_port.to_string()])
            .args(["-o", "StrictHostKeyChecking=no"])
            .args(["-o", "UserKnownHostsFile=/dev/null"])
            .args(["-v"]); // verbose mode so scp writes progress to stderr

        if !key_path.is_empty() {
            cmd.args(["-i", &key_path]);
        }

        match req_direction {
            TransferDirection::Upload => {
                cmd.arg(&req_local_path)
                    .arg(format!("{}@127.0.0.1:{}", username, req_remote_path));
            }
            TransferDirection::Download => {
                cmd.arg(format!("{}@127.0.0.1:{}", username, req_remote_path))
                    .arg(&req_local_path);
            }
        }

        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let mut scp_child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                error!("transfer {id_str}: failed to spawn scp: {e}");
                let mut p = progress_clone.lock().await;
                p.status = TransferStatus::Failed;
                p.error = Some(format!("Failed to start scp: {e}"));
                return;
            }
        };

        let stderr = match scp_child.stderr.take() {
            Some(s) => s,
            None => {
                let mut p = progress_clone.lock().await;
                p.status = TransferStatus::Failed;
                p.error = Some("Failed to capture scp stderr".into());
                return;
            }
        };

        // Store the scp child in state
        {
            let mut transfers = state_clone.transfers.lock().await;
            if let Some(t) = transfers.get_mut(&id_str) {
                t.scp_child = Some(scp_child);
            }
        }

        parse_scp_output(stderr, progress_clone.clone(), cancel_clone.clone()).await;

        // Wait for the scp child to finish
        let exit_ok = {
            let mut transfers = state_clone.transfers.lock().await;
            if let Some(t) = transfers.get_mut(&id_str) {
                if let Some(child) = t.scp_child.as_mut() {
                    matches!(child.wait().await, Ok(s) if s.success())
                } else {
                    false
                }
            } else {
                false
            }
        };

        {
            let mut p = progress_clone.lock().await;
            if p.status == TransferStatus::Transferring {
                if exit_ok {
                    p.status = TransferStatus::Completed;
                    p.percent = 100;
                } else {
                    p.status = TransferStatus::Failed;
                    if p.error.is_none() {
                        p.error = Some("scp exited with non-zero status".into());
                    }
                }
            }
        }

        // Clean up tunnel
        {
            let mut transfers = state_clone.transfers.lock().await;
            if let Some(t) = transfers.get_mut(&id_str)
                && let Some(child) = t.tunnel_child.as_mut()
            {
                let _ = child.kill().await;
            }
        }

        info!("transfer {id_str}: finished");
    });

    Json(serde_json::json!({ "id": transfer_id.to_string() })).into_response()
}

/// GET /api/transfer-progress/{transfer_id}
pub async fn get_transfer_progress(
    Path(transfer_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let transfers = state.transfers.lock().await;
    match transfers.get(&transfer_id) {
        Some(t) => {
            let progress = t.progress.lock().await.clone();
            Json(progress).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Transfer not found").into_response(),
    }
}

/// DELETE /api/transfer/{transfer_id}
pub async fn cancel_transfer(
    Path(transfer_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let mut transfers = state.transfers.lock().await;
    if let Some(t) = transfers.get_mut(&transfer_id) {
        t.cancel.store(true, Ordering::Relaxed);
        if let Some(child) = t.scp_child.as_mut() {
            let _ = child.kill().await;
        }
        if let Some(child) = t.tunnel_child.as_mut() {
            let _ = child.kill().await;
        }
        {
            let mut p = t.progress.lock().await;
            p.status = TransferStatus::Cancelled;
        }
        transfers.remove(&transfer_id);
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Transfer not found").into_response()
    }
}

/// Probes `127.0.0.1:port` every 500 ms for up to `timeout_secs` seconds.
/// Returns true when the port becomes reachable.
async fn probe_port(port: u16, timeout_secs: u64) -> bool {
    use tokio::net::TcpStream;
    use tokio::time::{Duration, Instant, sleep};

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Reads scp stderr, parses progress lines, and updates the shared `TransferProgress`.
/// SCP with `-v` writes lines like:
///   `filename   51% 512KB  512.0KB/s   00:01`
async fn parse_scp_output(
    mut stderr: tokio::process::ChildStderr,
    progress: Arc<tokio::sync::Mutex<TransferProgress>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) {
    let re = regex::Regex::new(r"(\d+)%\s+([\d.]+\s*\S+)\s+([\d.]+\s*\S+/s)\s*([\d:]+)?")
        .expect("static regex is valid");

    let mut buf = vec![0u8; 4096];
    let mut leftover = String::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                leftover.push_str(&String::from_utf8_lossy(&buf[..n]));
                let segments: Vec<&str> = leftover.split(['\r', '\n']).collect();
                let last = segments.len().saturating_sub(1);
                for (i, segment) in segments.iter().enumerate() {
                    if i == last {
                        // May be incomplete — keep as leftover
                        leftover = segment.to_string();
                        break;
                    }
                    if let Some(caps) = re.captures(segment) {
                        let mut p = progress.lock().await;
                        p.percent = caps[1].parse().unwrap_or(p.percent);
                        p.speed = caps
                            .get(3)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default();
                        p.eta = caps
                            .get(4)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default();
                    }
                }
            }
        }
    }
}
