//! SCP file transfer handler — upload/download files through an SSM port-forward tunnel on port 22.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;

use crate::handlers::connections::{
    find_free_port, find_key_path, lookup_instance_key_name, start_ssm_port_forward,
};
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

    // Validate user-supplied paths and username before any AWS call or tunnel start
    if let Err(msg) = validate_username(&username) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    if let Err(msg) = validate_remote_path(&req.remote_path) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    let local_path = match req.direction {
        TransferDirection::Upload => validate_local_path(&req.local_path, true),
        TransferDirection::Download => validate_local_path(&req.local_path, false),
    };
    let local_path = match local_path {
        Ok(p) => p,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };
    let strict_host_key = prefs.strict_host_key_checking;

    // Resolve SSH key path
    let key_path: String = if let Some(kp) = &req.key_path {
        if !kp.is_empty() {
            kp.clone()
        } else {
            String::new()
        }
    } else {
        // Auto-detect: look up the key name from EC2, then find the file
        let key_name = lookup_instance_key_name(&prefs, &instance_id).await;

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
    let req_local_path = local_path.to_string_lossy().into_owned();
    let req_remote_path = req.remote_path.clone();

    tokio::spawn(async move {
        // Probe the tunnel port (up to 15 s)
        let ready = probe_port(local_port, 15).await;
        if !ready || cancel_clone.load(Ordering::Relaxed) {
            let mut p = progress_clone.lock().await;
            p.status = TransferStatus::Failed;
            p.error = Some("SSH tunnel did not become reachable within 15 seconds".into());
            info!("transfer {id_str}: tunnel timeout");
            schedule_transfer_removal(state_clone.clone(), id_str.clone());
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
        cmd.args(["-P", &local_port.to_string()]).args([
            "-o",
            &format!(
                "StrictHostKeyChecking={}",
                if strict_host_key { "yes" } else { "no" }
            ),
        ]);
        if !strict_host_key {
            cmd.args(["-o", "UserKnownHostsFile=/dev/null"]);
        }

        if !key_path.is_empty() {
            cmd.args(["-i", &key_path]);
        }

        // `--` ends option parsing so paths can never be read as scp options
        cmd.arg("--");
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

        let mut stderr = match scp_child.stderr.take() {
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

        // Drain stderr concurrently so scp never blocks on a full pipe; kept for error reporting
        let stderr_task = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });

        // Poll for exit without holding the registry lock, so cancel_transfer can always acquire it
        let exit_ok = loop {
            let status = {
                let mut transfers = state_clone.transfers.lock().await;
                match transfers
                    .get_mut(&id_str)
                    .and_then(|t| t.scp_child.as_mut())
                {
                    Some(child) => child.try_wait(),
                    None => break false,
                }
            };
            match status {
                Ok(Some(s)) => break s.success(),
                Ok(None) => sleep(Duration::from_millis(200)).await,
                Err(_) => break false,
            }
        };
        let stderr_text = stderr_task.await.unwrap_or_default();
        let last_line = stderr_text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_owned);

        {
            let mut p = progress_clone.lock().await;
            if p.status == TransferStatus::Transferring {
                if exit_ok {
                    p.status = TransferStatus::Completed;
                    p.percent = 100;
                } else {
                    p.status = TransferStatus::Failed;
                    p.error =
                        Some(last_line.unwrap_or_else(|| "scp exited with non-zero status".into()));
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
        schedule_transfer_removal(state_clone.clone(), id_str.clone());
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

/// Validates a remote SSH username: 1-32 chars, ASCII alphanumerics plus `._-`, not starting with `-`.
/// Args:
///   username: Username to check.
/// Returns:
///   Ok(()) when valid, otherwise a static error message.
pub fn validate_username(username: &str) -> Result<(), &'static str> {
    if username.is_empty() || username.len() > 32 {
        return Err("username must be 1-32 characters");
    }
    if username.starts_with('-') {
        return Err("username must not start with '-'");
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err("username contains invalid characters");
    }
    Ok(())
}

/// Validates a remote path: 1-4096 chars, no leading `-`, no control characters.
/// Args:
///   path: Remote path supplied by the client.
/// Returns:
///   Ok(()) when valid, otherwise a static error message.
fn validate_remote_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > 4096 {
        return Err("remote path must be 1-4096 characters");
    }
    if path.starts_with('-') {
        return Err("remote path must not start with '-'");
    }
    if path.chars().any(|c| c.is_control()) {
        return Err("remote path contains control characters");
    }
    Ok(())
}

/// Resolves a local path (following symlinks) and ensures it lies inside the user's home directory.
/// Args:
///   path: Local path supplied by the client.
///   is_upload: True when the path is the source file (must exist as a file); false when it is
///     the download destination folder (must exist as a directory).
/// Returns:
///   The canonical path on success, otherwise a static error message.
fn validate_local_path(path: &str, is_upload: bool) -> Result<std::path::PathBuf, &'static str> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('-')
        || path.chars().any(|c| c.is_control())
    {
        return Err("invalid local path");
    }
    let canon = std::path::Path::new(path)
        .canonicalize()
        .map_err(|_| "local path does not exist")?;
    if is_upload && !canon.is_file() {
        return Err("upload source must be a file");
    }
    if !is_upload && !canon.is_dir() {
        return Err("download destination must be a folder");
    }
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .ok_or("home directory unavailable")?;
    let home = home.canonicalize().unwrap_or(home);
    if !canon.starts_with(&home) {
        return Err("local path must be inside your home directory");
    }
    Ok(canon)
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

/// Removes a finished transfer from the registry after a grace period, so the UI can still read its final status.
/// Args:
///   state: Application state holding the transfer registry.
///   id: Identifier of the finished transfer.
fn schedule_transfer_removal(state: AppState, id: String) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(600)).await;
        state.transfers.lock().await.remove(&id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_accepts_common_linux_users() {
        for u in ["ec2-user", "ubuntu", "admin.user_1", "a"] {
            assert!(validate_username(u).is_ok(), "should accept {u}");
        }
    }

    #[test]
    fn username_rejects_bad_input() {
        assert!(validate_username("").is_err());
        assert!(validate_username("-rf").is_err());
        assert!(validate_username("a b").is_err());
        assert!(validate_username("user;rm").is_err());
        assert!(validate_username(&"a".repeat(33)).is_err());
    }

    #[test]
    fn remote_path_accepts_absolute_paths() {
        assert!(validate_remote_path("/home/ec2-user/file.txt").is_ok());
        assert!(validate_remote_path("relative/path").is_ok());
    }

    #[test]
    fn remote_path_rejects_option_like_control_and_oversize() {
        assert!(validate_remote_path("").is_err());
        assert!(validate_remote_path("-oProxyCommand=x").is_err());
        assert!(validate_remote_path("a\nb").is_err());
        assert!(validate_remote_path("a\0b").is_err());
        assert!(validate_remote_path(&"x".repeat(4097)).is_err());
    }

    #[test]
    fn local_path_rejects_missing_and_option_like_paths() {
        assert!(validate_local_path("", true).is_err());
        assert!(validate_local_path("-x", true).is_err());
        assert!(validate_local_path("/definitely/not/here/xyz", true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn local_path_rejects_directories_outside_home() {
        // "/" exists and is a directory, but is never inside the home directory.
        assert!(validate_local_path("/", false).is_err());
    }

    #[test]
    fn local_path_accepts_home_directory_for_download() {
        if let Some(user) = directories::UserDirs::new() {
            let home = user.home_dir().to_string_lossy().into_owned();
            let resolved = validate_local_path(&home, false);
            assert!(
                resolved.is_ok(),
                "home dir should be a valid download folder"
            );
        }
    }

    #[test]
    fn upload_source_must_be_a_file_not_a_directory() {
        if let Some(user) = directories::UserDirs::new() {
            let home = user.home_dir().to_string_lossy().into_owned();
            assert!(validate_local_path(&home, true).is_err());
        }
    }
}
