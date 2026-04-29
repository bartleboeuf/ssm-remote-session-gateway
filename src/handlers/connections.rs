use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::models::{ConnectionInfo, ManagedConnection, RdpClientReq, StartPortReq, err, ok};
use crate::state::AppState;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_ec2 as ec2;
use aws_sdk_ssm as ssm;
use aws_types::region::Region;
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::time;
use tracing::{error, info, warn};
use which::which;

/// Finds a free local port within the specified range by attempting to bind to each port until it finds one that is available.
/// Args:
///   start: Starting port number of the range to check (inclusive)
///   end: Ending port number of the range to check (inclusive)
/// Returns:
///   Some(port) if a free port is found, or None if no free ports are available in the specified range.
/// Note: This function iterates through the specified port range and attempts to bind a TcpListener to each port. If the bind operation is successful, it means the port is
fn find_free_port(start: u16, end: u16) -> Option<u16> {
    (start..=end).find(|p| TcpListener::bind(("127.0.0.1", *p)).is_ok())
}

/// Validates that an SSM document exists and is accessible.
/// Returns detailed error if document is not found or inaccessible.
async fn validate_ssm_document(
    client: &ssm::Client,
    doc_name: &str,
) -> Result<(), (StatusCode, Json<crate::models::ErrorResponse>)> {
    match client.describe_document().name(doc_name).send().await {
        Ok(_) => Ok(()),
        Err(e) => {
            error!("SSM document validation failed for {}: {e}", doc_name);
            Err(crate::models::err(
                StatusCode::BAD_REQUEST,
                format!(
                    "SSM document '{}' not available. Ensure the instance has SSM agent running and your credentials have ssm:StartSession permission.",
                    doc_name
                ),
            ))
        }
    }
}

/// Starts an SSM port forwarding session by calling the AWS SSM StartSession API directly
/// via the SDK and then invoking the `session-manager-plugin` binary with the session token.
///
/// This avoids requiring the full AWS CLI to be installed — only `session-manager-plugin` is needed.
///
/// The flow mirrors what the AWS CLI does internally:
/// 1. Call SSM StartSession API to obtain a session token and WebSocket stream URL
/// 2. Invoke `session-manager-plugin` with the session response JSON, region, and parameters
async fn start_ssm_port_forward(
    info: &ConnectionInfo,
    prefs: &crate::models::Preferences,
) -> Result<tokio::process::Child, (StatusCode, Json<crate::models::ErrorResponse>)> {
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
    let region = prefs
        .last_region
        .clone()
        .or_else(|| std::env::var("AWS_REGION").ok())
        .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
        .unwrap_or_else(|| "us-east-1".into());

    tracing::info!(
        "Starting SSM port forward profile={} region={} instance={} local_port={} remote_port={} remote_host={:?}",
        profile,
        region,
        info.instance_id,
        info.local_port,
        info.remote_port,
        info.remote_host,
    );

    // --- Step 1: Build AWS config and SSM client ---
    let region_provider =
        RegionProviderChain::first_try(Some(Region::new(region.clone()))).or_default_provider();

    let cfg = aws_config::from_env()
        .profile_name(&profile)
        .region(region_provider)
        .load()
        .await;

    let ssm_client = ssm::Client::new(&cfg);

    // --- Step 2: Determine document name and parameters based on remote_host ---
    let (document_name, parameters, session_manager_params_json) =
        if let Some(ref remote_host) = info.remote_host {
            // Port forwarding to a remote host (e.g. RDS endpoint)
            let doc = "AWS-StartPortForwardingSessionToRemoteHost";
            let mut params: HashMap<String, Vec<String>> = HashMap::new();
            params.insert("host".into(), vec![remote_host.clone()]);
            params.insert("portNumber".into(), vec![info.remote_port.to_string()]);
            params.insert("localPortNumber".into(), vec![info.local_port.to_string()]);
            let sm_params = serde_json::json!({
                "Target": info.instance_id,
                "DocumentName": doc,
                "parameters": {
                    "host": [remote_host],
                    "portNumber": [info.remote_port.to_string()],
                    "localPortNumber": [info.local_port.to_string()]
                }
            });
            (doc, params, sm_params)
        } else {
            // Standard port forwarding through the instance
            let doc = "AWS-StartPortForwardingSession";
            let mut params: HashMap<String, Vec<String>> = HashMap::new();
            params.insert("portNumber".into(), vec![info.remote_port.to_string()]);
            params.insert("localPortNumber".into(), vec![info.local_port.to_string()]);
            let sm_params = serde_json::json!({
                "Target": info.instance_id,
                "DocumentName": doc,
                "parameters": {
                    "portNumber": [info.remote_port.to_string()],
                    "localPortNumber": [info.local_port.to_string()]
                }
            });
            (doc, params, sm_params)
        };

    // --- Step 3: Validate SSM document exists ---
    validate_ssm_document(&ssm_client, document_name).await?;

    // --- Step 4: Call SSM StartSession API ---
    tracing::info!(
        "Calling SSM StartSession API: target={} document={} parameters={:?}",
        info.instance_id,
        document_name,
        parameters,
    );

    let start_session_resp = ssm_client
        .start_session()
        .target(&info.instance_id)
        .document_name(document_name)
        .set_parameters(Some(parameters))
        .send()
        .await
        .map_err(|e| {
            tracing::error!("SSM StartSession API call failed: {e}");
            err(
                StatusCode::BAD_REQUEST,
                format!("SSM StartSession failed: {e}. Ensure the instance has SSM agent running and your credentials have ssm:StartSession permission."),
            )
        })?;

    let session_id = start_session_resp
        .session_id()
        .unwrap_or_default()
        .to_string();
    let token_value = start_session_resp
        .token_value()
        .unwrap_or_default()
        .to_string();
    let stream_url = start_session_resp
        .stream_url()
        .unwrap_or_default()
        .to_string();

    tracing::info!("SSM session started: session_id={}", session_id);

    let response_json = serde_json::json!({
        "SessionId": session_id,
        "TokenValue": token_value,
        "StreamUrl": stream_url,
    });

    // --- Step 4: Invoke session-manager-plugin ---
    let endpoint = format!("https://ssm.{}.amazonaws.com", region);

    tracing::info!(
        "Invoking session-manager-plugin: session_id={} region={} endpoint={}",
        session_id,
        region,
        endpoint,
    );

    let mut cmd = TokioCommand::new("session-manager-plugin");
    cmd.arg(response_json.to_string())
        .arg(&region)
        .arg("StartSession")
        .arg(&profile)
        .arg(session_manager_params_json.to_string())
        .arg(&endpoint)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        tracing::error!("Failed to start session-manager-plugin: {e}");
        err(
            StatusCode::BAD_REQUEST,
            format!(
                "Failed to start session-manager-plugin: {e}. \
                 Install it from https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html"
            ),
        )
    })?;

    // Stream stdout/stderr to logs
    if let Some(mut out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut buf = String::new();
            if out.read_to_string(&mut buf).await.is_ok() && !buf.trim().is_empty() {
                for line in buf.lines() {
                    tracing::info!("[ssm stdout] {}", line);
                }
            }
        });
    }
    if let Some(mut errp) = child.stderr.take() {
        tokio::spawn(async move {
            let mut buf = String::new();
            if errp.read_to_string(&mut buf).await.is_ok() && !buf.trim().is_empty() {
                for line in buf.lines() {
                    tracing::error!("[ssm stderr] {}", line);
                }
            }
        });
    }

    // Brief check for immediate failure
    time::sleep(Duration::from_millis(500)).await;
    if let Ok(Some(status)) = child.try_wait() {
        tracing::error!("session-manager-plugin exited early (status {})", status,);
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!(
                "session-manager-plugin exited ({status}). \
                 Ensure session-manager-plugin is installed and up to date."
            ),
        ));
    }

    // Spawn a timeout task that logs if the process hangs for 5 minutes.
    // Connection cleanup happens when child process exits or is dropped.
    tokio::spawn(async move {
        time::sleep(Duration::from_secs(300)).await;
        tracing::warn!(
            "SSM port-forward idle for 5 minutes: session_id={}",
            session_id
        );
    });

    Ok(child)
}

/// Build rich connection info used by the frontend for user instructions.
/// Constructs the JSON payload describing how the frontend should use the forwarded port.
/// Represents the data needed to describe a forwarded port session to the frontend.
struct ConnectionInfoArgs<'a> {
    instance_id: &'a str,
    remote_port: u16,
    local_port: u16,
    remote_host: Option<&'a str>,
    ssh_options: &'a str,
    key_name: Option<&'a str>,
    key_path: Option<&'a str>,
    linux_username: &'a str,
    windows_username: &'a str,
}

/// Formats a user instruction payload describing the local and remote endpoints.
fn build_connection_info(args: ConnectionInfoArgs) -> serde_json::Value {
    let ConnectionInfoArgs {
        instance_id,
        remote_port,
        local_port,
        remote_host,
        ssh_options,
        key_name,
        key_path,
        linux_username,
        windows_username,
    } = args;
    let ip = "127.0.0.1";
    match (remote_port, remote_host) {
        (22, _) => {
            let command = format!("ssh {ssh_options} -p {local_port} {linux_username}@{ip}");
            let mut command_with_key = command.clone();
            if let Some(path) = key_path {
                let quoted = format!("\"{}\"", path);
                command_with_key =
                    format!("ssh {ssh_options} -i {quoted} -p {local_port} {linux_username}@{ip}");
            }
            serde_json::json!({
                "type": "ssh",
                "instruction": format!("Connect via SSH to {ip}:{local_port}"),
                "command": command,
                "ssh_command_with_key": command_with_key,
                "note": format!("Replace '{linux_username}' with the appropriate username (ubuntu, admin, etc.) if needed"),
                "ip": ip,
                "port": local_port.to_string(),
                "user": linux_username,
                "key_name": key_name,
                "key_path": key_path,
            })
        }
        (3389, _) => serde_json::json!({
            "type": "rdp",
            "instance_id": instance_id,
            "instruction": format!("Connect via RDP to {ip}:{local_port}"),
            "address": format!("{ip}:{local_port}"),
            "note": "Use your RDP client (mstsc, Microsoft Remote Desktop, etc.) to connect",
            "ip": ip,
            "port": local_port.to_string(),
            "user": windows_username,
            "key_name": key_name
        }),
        (_, Some(host)) => serde_json::json!({
            "type": "remote_host_port",
            "instruction": format!("Port forwarding active on {ip}:{local_port} -> {host}:{remote_port}"),
            "local_address": format!("{ip}:{local_port}"),
            "remote_port": remote_port.to_string(),
            "remote_host": host,
            "ip": ip,
            "port": local_port.to_string()
        }),
        _ => serde_json::json!({
            "type": "custom_port",
            "instruction": format!("Port forwarding active on {ip}:{local_port}"),
            "local_address": format!("{ip}:{local_port}"),
            "remote_port": remote_port.to_string(),
            "ip": ip,
            "port": local_port.to_string()
        }),
    }
}

#[derive(Debug, Deserialize)]
pub struct ExecuteSshRequest {
    pub command: String,
    pub connection_id: Option<uuid::Uuid>,
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn escape_single_quotes_for_bash(input: &str) -> String {
    input.replace("'", "'\\''")
}

fn launch_ssh_in_terminal(
    command: &str,
) -> Result<(), (StatusCode, Json<crate::models::ErrorResponse>)> {
    if command.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "SSH command is empty"));
    }
    if !command.trim_start().starts_with("ssh") {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Only SSH commands may be executed from the web UI",
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let escaped = command.replace('"', "\\\"");
        let apple_script = format!("tell application \"Terminal\" to do script \"{}\"", escaped);
        Command::new("osascript")
            .arg("-e")
            .arg(apple_script)
            .spawn()
            .map_err(|e| {
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to launch Terminal: {e}"),
                )
            })?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let pwsh_command = format!("& {{ {} }}", command.replace('"', "\\\""));
        let mut launcher = if which("wt.exe").is_ok() {
            let mut cmd = Command::new("wt.exe");
            cmd.args([
                "new-tab",
                "powershell.exe",
                "-NoExit",
                "-Command",
                &pwsh_command,
            ]);
            cmd
        } else {
            let mut cmd = Command::new("cmd");
            cmd.args([
                "/C",
                "start",
                "",
                "powershell.exe",
                "-NoExit",
                "-Command",
                &pwsh_command,
            ]);
            cmd
        };
        launcher.spawn().map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to launch Windows terminal: {e}"),
            )
        })?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let bash_command = format!("{}; exec bash", escape_single_quotes_for_bash(command));
        let terminals = [
            (
                "x-terminal-emulator",
                vec!["-e", "bash", "-lc", bash_command.as_str()],
            ),
            (
                "gnome-terminal",
                vec!["--", "bash", "-lc", bash_command.as_str()],
            ),
            ("konsole", vec!["-e", "bash", "-lc", bash_command.as_str()]),
            (
                "alacritty",
                vec!["-e", "bash", "-lc", bash_command.as_str()],
            ),
            (
                "terminator",
                vec!["-x", "bash", "-lc", bash_command.as_str()],
            ),
            (
                "lxterminal",
                vec!["-e", "bash", "-lc", bash_command.as_str()],
            ),
            ("xterm", vec!["-e", "bash", "-lc", bash_command.as_str()]),
        ];

        for (terminal, args) in terminals.iter() {
            if which(terminal).is_ok() {
                let mut cmd = Command::new(terminal);
                cmd.args(args);
                if cmd.spawn().is_ok() {
                    return Ok(());
                }
            }
        }

        Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No supported terminal emulator was found. Install a terminal emulator like gnome-terminal, xterm, alacritty, or configure a launcher in preferences.",
        ))
    }
}

pub async fn execute_ssh_command(
    Json(req): Json<ExecuteSshRequest>,
) -> Result<
    Json<crate::models::ApiResponse<serde_json::Value>>,
    (StatusCode, Json<crate::models::ErrorResponse>),
> {
    launch_ssh_in_terminal(&req.command)?;
    Ok(ok(serde_json::json!({
        "message": "SSH command launched in local terminal",
        "connection_id": req.connection_id,
    })))
}

/// Retrieves the key pair name attached to the given EC2 instance via DescribeInstances.
async fn lookup_instance_key_name(
    prefs: &crate::models::Preferences,
    instance_id: &str,
) -> Option<String> {
    let profile = prefs
        .last_profile
        .clone()
        .unwrap_or_else(|| "default".into());
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
        .instance_ids(instance_id)
        .send()
        .await
    {
        Ok(resp) => resp
            .reservations()
            .iter()
            .flat_map(|r| r.instances())
            .find_map(|i| i.key_name().map(|s| s.to_string())),
        Err(e) => {
            warn!("Could not fetch key name for instance {instance_id}: {e}");
            None
        }
    }
}

/// Searches configured folders (including ~/.ssh) for a PEM/key file matching the key name.
fn find_key_path(key_name: &str, folders: Option<&str>) -> Option<String> {
    let mut dirs = Vec::new();
    if let Some(f) = folders {
        for part in f.split([',', '\n']) {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let path = if trimmed.starts_with('~') {
                directories::UserDirs::new()
                    .map(|u| u.home_dir().join(trimmed.trim_start_matches('~')))
            } else {
                Some(std::path::PathBuf::from(trimmed))
            };
            if let Some(p) = path {
                dirs.push(p);
            }
        }
    }
    // fallback ~/.ssh
    if let Some(user_dirs) = directories::UserDirs::new() {
        dirs.push(user_dirs.home_dir().join(".ssh"));
    }

    let candidates = [
        key_name.to_string(),
        format!("{key_name}.pem"),
        format!("{key_name}.key"),
    ];

    for dir in dirs {
        for cand in &candidates {
            let path = dir.join(cand);
            if path.exists() {
                return path.to_str().map(|s| s.replace('\\', "/"));
            }
        }
    }
    None
}

/// Starts a port forwarding session for the specified EC2 instance and remote port, optionally using a
/// specified local port and remote host.
/// Args:
///   instance_id: The ID of the EC2 instance to connect to (e.g. "i-0123456789abcdef0")
///   req: A StartPortReq struct containing the remote port to forward, an optional local port to use
///        (if not specified, a free port will be automatically selected), and an optional remote host
///        to connect to (defaults to "localhost" if not specified)
///   state: The application state containing preferences and active connections
/// Returns:
///   On success, returns a JSON response containing the connection ID and connection information
///   (including the local port that was used). On failure, returns an error response with an appropriate
///   HTTP status code and error message (e.g. if no free local ports are available, or if the instance ID is
///   invalid). The function will attempt to find a free local port if one is not specified in the request,
///   and will create a new ConnectionInfo struct to represent the active connection, which is then stored
///   in the application state for tracking. The connection information includes the command that can be used
///   to start the port forwarding session using AWS SSM.  
/// Note: The actual implementation of starting the port forwarding session (e.g. invoking the AWS CLI or SDK
/// to start the session) is not included in this function, as it is assumed that the client will use
/// the provided connection information to initiate the session separately. This function is primarily
/// responsible for managing the connection information and ensuring that a free local port is allocated
/// for the session.
async fn start_port(
    instance_id: String,
    req: StartPortReq,
    State(state): State<AppState>,
) -> Result<
    Json<crate::models::ApiResponse<serde_json::Value>>,
    (StatusCode, Json<crate::models::ErrorResponse>),
> {
    let prefs = state.prefs.read().await.clone();
    let local = req
        .local_port
        .or_else(|| find_free_port(prefs.port_range_start, prefs.port_range_end))
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "No free port available"))?;

    let key_name = lookup_instance_key_name(&prefs, &instance_id).await;
    let key_path = key_name
        .as_deref()
        .and_then(|k| find_key_path(k, prefs.ssh_key_folder.as_deref()));

    let info = ConnectionInfo {
        id: uuid::Uuid::new_v4(),
        profile: prefs
            .last_profile
            .clone()
            .unwrap_or_else(|| "default".into()),
        region: prefs
            .last_region
            .clone()
            .unwrap_or_else(|| "us-east-1".into()),
        instance_id,
        local_port: local,
        remote_port: req.remote_port,
        remote_host: req.remote_host.clone(),
        command: format!(
            "aws ssm start-session --target <instance> --document-name AWS-StartPortForwardingSession --parameters portNumber={},localPortNumber={}",
            req.remote_port, local
        ),
        key_name,
        key_path: key_path.clone(),
    };

    let child = start_ssm_port_forward(&info, &prefs).await?;
    tracing::info!(
        "Started port forward: instance={} local_port={} remote_port={} key_name={:?}",
        info.instance_id,
        info.local_port,
        info.remote_port,
        info.key_name
    );
    state.connections.lock().await.insert(
        info.id,
        ManagedConnection {
            info: info.clone(),
            child: Some(child),
        },
    );

    let connection_info = build_connection_info(ConnectionInfoArgs {
        instance_id: &info.instance_id,
        remote_port: req.remote_port,
        local_port: local,
        remote_host: req.remote_host.as_deref(),
        ssh_options: &prefs.ssh_options,
        key_name: info.key_name.as_deref(),
        key_path: info.key_path.as_deref(),
        linux_username: &prefs.linux_username,
        windows_username: &prefs.windows_username,
    });

    Ok(ok(serde_json::json!({
        "connection_id": info.id,
        "local_port": local,
        "remote_port": req.remote_port,
        "command": info.command,
        "connection_info": connection_info,
    })))
}

/// Starts a standard SSH tunnel for the given EC2 instance.
pub async fn ssh(Path(instance_id): Path<String>, state: State<AppState>) -> impl IntoResponse {
    start_port(
        instance_id,
        StartPortReq {
            remote_port: 22,
            local_port: None,
            remote_host: None,
        },
        state,
    )
    .await
}

/// Starts an RDP tunnel for the given EC2 instance.
pub async fn rdp(Path(instance_id): Path<String>, state: State<AppState>) -> impl IntoResponse {
    start_port(
        instance_id,
        StartPortReq {
            remote_port: 3389,
            local_port: None,
            remote_host: None,
        },
        state,
    )
    .await
}

/// Starts a custom port-forwarding session using the provided request body.
pub async fn custom_port(
    Path(instance_id): Path<String>,
    state: State<AppState>,
    Json(req): Json<StartPortReq>,
) -> impl IntoResponse {
    start_port(instance_id, req, state).await
}

/// Terminates a single active connection by ID.
pub async fn terminate(
    Path(conn_id): Path<uuid::Uuid>,
    State(state): State<AppState>,
) -> Result<
    Json<crate::models::ApiResponse<serde_json::Value>>,
    (StatusCode, Json<crate::models::ErrorResponse>),
> {
    let removed = state.connections.lock().await.remove(&conn_id);
    if let Some(mut mc) = removed {
        if let Some(mut child) = mc.child.take() {
            let _ = child.kill().await;
        }
        tracing::info!("Terminated connection {}", conn_id);
        Ok(ok(serde_json::json!({})))
    } else {
        Err(err(StatusCode::NOT_FOUND, "Connection not found"))
    }
}

/// Terminates all active connections.
pub async fn terminate_all(State(state): State<AppState>) -> impl IntoResponse {
    let mut lock = state.connections.lock().await;
    for mc in lock.values_mut() {
        if let Some(mut child) = mc.child.take() {
            let _ = child.kill().await;
        }
    }
    tracing::info!("Terminated all active connections");
    lock.clear();
    ok(serde_json::json!({"message": "All connections terminated"}))
}

/// Returns the list of currently active connections.
pub async fn active(State(state): State<AppState>) -> impl IntoResponse {
    let prefs = state.prefs.read().await.clone();
    let vals: Vec<_> = state
        .connections
        .lock()
        .await
        .values()
        .map(|mc| {
            let connection_info = build_connection_info(ConnectionInfoArgs {
                instance_id: &mc.info.instance_id,
                remote_port: mc.info.remote_port,
                local_port: mc.info.local_port,
                remote_host: mc.info.remote_host.as_deref(),
                ssh_options: &prefs.ssh_options,
                key_name: mc.info.key_name.as_deref(),
                key_path: mc.info.key_path.as_deref(),
                linux_username: &prefs.linux_username,
                windows_username: &prefs.windows_username,
            });
            serde_json::json!({
                "connection_id": mc.info.id,
                "id": mc.info.id,
                "profile": mc.info.profile,
                "region": mc.info.region,
                "instance_id": mc.info.instance_id,
                "local_port": mc.info.local_port,
                "remote_port": mc.info.remote_port,
                "remote_host": mc.info.remote_host,
                "command": mc.info.command,
                "key_name": mc.info.key_name,
                "key_path": mc.info.key_path,
                "connection_info": connection_info,
            })
        })
        .collect();
    Json(vals)
}

/// Signal the server to shut down gracefully.
pub async fn quit(State(state): State<AppState>) -> impl IntoResponse {
    let _ = state.shutdown_tx.send(true);
    ok(serde_json::json!({"message": "Shutting down"}))
}

/// Launches an RDP client on the local machine.
pub async fn rdp_client_connect(
    Json(req): Json<RdpClientReq>,
) -> Result<
    Json<crate::models::ApiResponse<serde_json::Value>>,
    (StatusCode, Json<crate::models::ErrorResponse>),
> {
    let req_for_log = req.clone();
    if req.ip.is_empty() || req.username.is_empty() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "IP, port, and username are required",
        ));
    }
    if !(1..=65_535).contains(&req.port) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Port must be between 1 and 65535",
        ));
    }

    let message = tokio::task::spawn_blocking(move || launch_rdp_client(&req))
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "RDP launch task failed"))??;
    info!(
        "RDP launch success for ip={} port={}",
        req_for_log.ip, req_for_log.port
    );

    Ok(ok(serde_json::json!({ "message": message })))
}

/// Launches a platform-specific RDP client for the supplied request.
fn launch_rdp_client(
    req: &RdpClientReq,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    info!(
        "Launching RDP client ip={} port={} user={} has_password={}",
        req.ip,
        req.port,
        req.username,
        req.password
            .as_ref()
            .map(|p| !p.is_empty())
            .unwrap_or(false)
    );
    match std::env::consts::OS {
        "windows" => launch_rdp_windows(req),
        "macos" => launch_rdp_macos(req),
        "linux" => launch_rdp_linux(req),
        other => Err(err(
            StatusCode::BAD_REQUEST,
            format!("Unsupported operating system: {other}"),
        )),
    }
}

/// Writes a temporary RDP configuration file used by the OS client.
fn build_rdp_file(
    ip: &str,
    port: u16,
    username: &str,
    password: &Option<String>,
) -> Result<PathBuf, std::io::Error> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "ssm-remote-session-gateway-{}.rdp",
        uuid::Uuid::new_v4()
    ));

    let mut contents = format!("full address:s:{ip}:{port}\nusername:s:{username}\n");
    if let Some(pw) = password {
        contents.push_str(&format!("password 51:b:{pw}\n"));
    }
    info!("Writing temporary RDP file at {:?}", path);
    std::fs::write(&path, contents)?;
    Ok(path)
}

/// Launches `mstsc` on Windows using a temporary RDP file.
fn launch_rdp_windows(
    req: &RdpClientReq,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    let path = build_rdp_file(&req.ip, req.port, &req.username, &None).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create RDP file: {e}"),
        )
    })?;

    let path_str = path
        .to_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid RDP file path"))?;

    let cmdline = format!("cmd /C start \"\" \"{}\"", path_str);
    info!("RDP windows launching: {}", cmdline);
    Command::new("cmd")
        .args(["/C", "start", "", path_str])
        .spawn()
        .map_err(|e| {
            error!("RDP windows launch failed: {}", e);
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to launch mstsc: {e}"),
            )
        })?;

    schedule_cleanup(path);
    Ok(format!(
        "RDP client launched connecting to {}:{} as {} (enter password when prompted)",
        req.ip, req.port, req.username
    ))
}

/// Attempts to open macOS Remote Desktop variants via the generated RDP file.
fn launch_rdp_macos(
    req: &RdpClientReq,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    let path = build_rdp_file(&req.ip, req.port, &req.username, &req.password).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create RDP file: {e}"),
        )
    })?;

    let path_str = path
        .to_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid RDP file path"))?;

    let try_apps = ["Windows App", "Microsoft Remote Desktop", "open"];
    let mut last_err = None;
    for app in try_apps {
        let cmdline = if app == "open" {
            format!("open \"{}\"", path_str)
        } else {
            format!("open -a \"{}\" \"{}\"", app, path_str)
        };
        info!("RDP macOS attempting: {}", cmdline);
        let result = if app == "open" {
            Command::new("open").arg(path_str).spawn()
        } else {
            Command::new("open").args(["-a", app, path_str]).spawn()
        };

        match result {
            Ok(_) => {
                schedule_cleanup(path);
                return Ok(format!(
                    "RDP client launched connecting to {}:{}",
                    req.ip, req.port
                ));
            }
            Err(e) => {
                error!("RDP macOS launch with {} failed: {}", app, e);
                last_err = Some(e)
            }
        }
    }

    Err(err(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!(
            "Failed to launch RDP client. Tried Windows App / Microsoft Remote Desktop. {:?}",
            last_err
        ),
    ))
}

/// Tries xfreerdp or remmina on Linux, falling back to WSL if needed.
fn launch_rdp_linux(
    req: &RdpClientReq,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    if let Ok(path) = which("xfreerdp") {
        let mut args = vec![
            format!("/v:{}:{}", req.ip, req.port),
            format!("/u:{}", req.username),
            "/cert:ignore".into(),
        ];
        if let Some(pw) = &req.password {
            args.push(format!("/p:{pw}"));
        }
        info!(
            "RDP linux launching xfreerdp: {} {:?}",
            path.display(),
            args
        );
        std::process::Command::new(path)
            .args(args)
            .spawn()
            .map_err(|e| {
                error!("Failed to start xfreerdp: {}", e);
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to start xfreerdp: {e}"),
                )
            })?;
        return Ok(format!(
            "RDP client (xfreerdp) launched connecting to {}:{}",
            req.ip, req.port
        ));
    }

    if let Ok(path) = which("remmina") {
        let mut conn = format!("rdp://{}@{}:{}", req.username, req.ip, req.port);
        if let Some(pw) = &req.password {
            conn = format!("rdp://{}:{}@{}:{}", req.username, pw, req.ip, req.port);
        }
        info!(
            "RDP linux launching remmina: {} -c {}",
            path.display(),
            conn
        );
        std::process::Command::new(path)
            .args(["-c", &conn])
            .spawn()
            .map_err(|e| {
                error!("Failed to start remmina: {}", e);
                err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to start remmina: {e}"),
                )
            })?;
        return Ok(format!(
            "RDP client (remmina) launched connecting to {}:{}",
            req.ip, req.port
        ));
    }

    if is_wsl() {
        info!("No native Linux RDP client found; attempting Windows mstsc via WSL");
        return launch_rdp_windows_from_wsl(req);
    }

    Err(err(
        StatusCode::BAD_REQUEST,
        "No RDP client found. Install xfreerdp or remmina.",
    ))
}

/// Deletes the temporary RDP file after a short delay to keep the temp dir tidy.
fn schedule_cleanup(path: PathBuf) {
    tokio::spawn(async move {
        time::sleep(Duration::from_secs(10)).await;
        let _ = std::fs::remove_file(path);
    });
}

/// Returns true when running under Windows Subsystem for Linux.
fn is_wsl() -> bool {
    std::env::var("WSL_INTEROP").is_ok()
        || fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.to_lowercase().contains("microsoft"))
            .unwrap_or(false)
}

/// Invokes Windows `mstsc` from WSL by translating the temp file path.
fn launch_rdp_windows_from_wsl(
    req: &RdpClientReq,
) -> Result<String, (StatusCode, Json<crate::models::ErrorResponse>)> {
    let path = build_rdp_file(&req.ip, req.port, &req.username, &req.password).map_err(|e| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create RDP file: {e}"),
        )
    })?;

    let linux_path = path
        .to_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid RDP file path"))?;

    // Convert to Windows path using wslpath
    let output = Command::new("wslpath")
        .args(["-w", linux_path])
        .output()
        .map_err(|e| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("wslpath failed: {e}"),
            )
        })?;

    if !output.status.success() {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("wslpath error: {}", String::from_utf8_lossy(&output.stderr)),
        ));
    }

    let win_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let cmdline = format!("cmd.exe /C start \"\" \"{}\"", win_path);
    info!("RDP WSL launching: {}", cmdline);
    Command::new("cmd.exe")
        .args(["/C", "start", "", &win_path])
        .spawn()
        .map_err(|e| {
            error!("Failed to launch mstsc via WSL: {}", e);
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to launch mstsc via WSL: {e}"),
            )
        })?;

    schedule_cleanup(path);
    Ok(format!(
        "RDP client (mstsc via WSL) launched connecting to {}:{}",
        req.ip, req.port
    ))
}
