use axum::{BoxError, error_handling::HandleErrorLayer};
use axum::{
    Router,
    http::{HeaderValue, Method},
    routing::{get, post},
};
use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use rustls::{
    ServerConfig, pki_types::CertificateDer, pki_types::PrivateKeyDer, pki_types::pem::PemObject,
};
use static_serve::embed_assets;
use std::sync::OnceLock;
use std::{collections::HashMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use tokio_rustls::TlsAcceptor;
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer, timeout::TimeoutLayer};
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use tracing::{info, warn};

use crate::handlers::transfer;
use crate::handlers::{aws, connections, prefs};
use crate::models::Preferences;
use crate::state::AppState;

embed_assets!(
    "src/static",
    compress = false,
    allow_unknown_extensions = false
);

/// Per-process API token. Generated at startup and required on every `/api` request.
static API_TOKEN: OnceLock<String> = OnceLock::new();

/// Header carrying the API token.
const API_TOKEN_HEADER: &str = "x-api-token";

/// Hosts accepted in the Host header (defends against DNS rebinding).
const ALLOWED_HOSTS: [&str; 2] = ["127.0.0.1:5000", "localhost:5000"];

/// Generates a random 256-bit token encoded as lowercase hex.
/// Returns:
///   Ok(token) with 64 hex characters, or an error if the system RNG fails.
fn generate_api_token() -> anyhow::Result<String> {
    use aws_lc_rs::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("failed to generate API token"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares two byte slices without early exit on the first mismatching byte.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Middleware: rejects `/api` requests that do not carry the correct `X-Api-Token` header.
/// Args:
///   req: Incoming request.
///   next: Remaining middleware and handler chain.
/// Returns:
///   The downstream response, or 401 when the token is missing or wrong.
async fn require_api_token(req: Request, next: Next) -> Response {
    let expected = API_TOKEN.get().map(String::as_bytes).unwrap_or_default();
    let provided = req
        .headers()
        .get(API_TOKEN_HEADER)
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if !expected.is_empty() && constant_time_eq(expected, provided) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "Missing or invalid API token").into_response()
    }
}

/// Middleware: rejects requests whose Host (or HTTP/2 authority) is not a loopback address on port 5000.
/// Args:
///   req: Incoming request.
///   next: Remaining middleware and handler chain.
/// Returns:
///   The downstream response, or 403 when the host is not allowed.
async fn check_host_header(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.as_str().to_owned()));
    match host {
        Some(h) if ALLOWED_HOSTS.contains(&h.as_str()) => next.run(req).await,
        _ => (StatusCode::FORBIDDEN, "Invalid Host header").into_response(),
    }
}

/// Content-Security-Policy for the UI. The page has no inline scripts or `style=` attributes.
/// Allowed origins: jsDelivr (Bootstrap CSS, icons, JS bundle), Google Fonts (font stylesheet
/// and font files), and api.github.com (plugin version check, via fetch).
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
    script-src 'self' https://cdn.jsdelivr.net; \
    style-src 'self' https://cdn.jsdelivr.net https://fonts.googleapis.com; \
    font-src 'self' https://cdn.jsdelivr.net https://fonts.gstatic.com; \
    img-src 'self' data:; \
    connect-src 'self' https://api.github.com; \
    object-src 'none'; \
    base-uri 'self'; \
    form-action 'self'; \
    frame-ancestors 'none'";

/// Serves the SPA shell with the per-process API token injected into the page.
async fn index_page() -> impl IntoResponse {
    let token = API_TOKEN.get().map(String::as_str).unwrap_or_default();
    let page = include_str!("static/templates/index.html").replace("__API_TOKEN__", token);
    Html(page)
}

/// Cleans up active connections by killing any child processes and clearing the connections map. This is typically called during server shutdown to ensure that all resources are properly released and there are no lingering processes left running after the server has stopped. It acquires a lock on the connections map, iterates through all managed connections, and attempts to kill any active child processes associated with those connections. After attempting to kill the processes, it clears the connections map to remove all entries. This function is designed to be robust and handle any errors that may occur during process termination without panicking, ensuring a clean shutdown process.
/// Args:
///   state: Reference to the shared application state, which contains the connections map that needs to be cleaned up. The function will acquire a lock on this map to safely access and modify it during the cleanup process.
/// Returns:
///   None. This function performs its cleanup operations and does not return any value. It is intended to be called as part of the server shutdown sequence and does not need to provide any output to the caller. Any errors encountered during the cleanup process (e.g. issues killing child processes) are logged but do not cause the function to return an error, allowing the shutdown process to continue even if there are issues with individual connections.
async fn cleanup_connections(state: &AppState) {
    let mut lock = state.connections.lock().await;
    for mc in lock.values_mut() {
        if let Some(mut child) = mc.child.take() {
            let _ = child.kill().await;
        }
    }
    lock.clear();

    // Stop any in-flight file transfers and their SSM tunnels
    let mut transfers = state.transfers.lock().await;
    for t in transfers.values_mut() {
        t.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(child) = t.scp_child.as_mut() {
            let _ = child.kill().await;
        }
        if let Some(child) = t.tunnel_child.as_mut() {
            let _ = child.kill().await;
        }
    }
    transfers.clear();
    tracing::info!("Cleaned up active connections and transfers");
}

/// Runs the Axum server with the specified initial preferences. This function sets up the application state, configures the API routes and middleware layers, and starts listening for incoming connections on a specified address. It also handles graceful shutdown when a Ctrl+C signal is received, ensuring that all active connections are cleaned up properly before the server exits. The server is configured to use TLS with a self-signed certificate for secure communication, and it includes CORS settings to allow requests from localhost origins. If any errors occur during server startup or runtime, they are logged appropriately.
/// Args:
///   initial_prefs: The initial preferences to load into the application state. This allows the server to start with a predefined configuration, which can be useful for testing or for providing default settings on first run. The preferences are stored in a shared, thread-safe manner using an Arc and RwLock, allowing them to be accessed and modified by the various handlers that need to read or update the preferences during the server's operation.
/// Returns:
///   Ok(()) if the server runs successfully and shuts down gracefully when a shutdown signal is received. If there is an error during server startup (e.g. issues with TLS configuration, binding  to the address) or during runtime (e.g. unhandled errors in request processing), it returns an Err with details about the failure. The error message will include information about what went wrong to help diagnose issues with server setup or operation.
/// Raises:
///   IOError: If there is an error reading the TLS certificate or key files from disk, or if there is an error binding to the specified address for the server to listen on.
///   TLSConfiguration Error: If there is an error generating a self-signed certificate or configuring the TLS settings for the server.
///   ServerRuntimeError: If there is an unhandled error during request processing or if there is an issue with the server's runtime environment that causes it to fail during operation.
pub async fn run(initial_prefs: Preferences, use_https: bool) -> anyhow::Result<()> {
    let token = generate_api_token()?;
    let _ = API_TOKEN.set(token);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let state = AppState {
        prefs: Arc::new(tokio::sync::RwLock::new(initial_prefs)),
        connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        instance_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        instance_refreshes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        rds_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        rds_refreshes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        ecs_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        ecs_refreshes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        transfers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        accounts: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        shutdown_tx: shutdown_tx.clone(),
    };

    aws::load_instance_cache_from_disk(&state).await;

    // Startup checks for required external tools
    if which::which("session-manager-plugin").is_ok() {
        tracing::info!("session-manager-plugin found in PATH");
    } else {
        tracing::warn!("session-manager-plugin NOT found in PATH — SSM sessions will not work");
    }
    // if which::which("aws").is_ok() {
    //     tracing::info!("AWS CLI found in PATH");
    // } else {
    //     tracing::warn!("AWS CLI NOT found in PATH — AWS operations will not work");
    // }

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_origin([
            "https://127.0.0.1:5000".parse::<HeaderValue>().unwrap(),
            "https://localhost:5000".parse().unwrap(),
            "http://127.0.0.1:5000".parse().unwrap(),
            "http://localhost:5000".parse().unwrap(),
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let api = Router::new()
        .route("/profiles", get(aws::list_profiles))
        .route("/regions", get(aws::list_regions))
        .route("/connect", post(aws::connect))
        .route("/instances", get(aws::list_instances))
        .route("/rds", get(aws::list_rds_databases))
        .route("/ecs", get(aws::list_ecs_tasks))
        .route("/ssh/{instance_id}", post(connections::ssh))
        .route("/rdp/{instance_id}", post(connections::rdp))
        .route("/custom-port/{instance_id}", post(connections::custom_port))
        .route("/execute-ssh", post(connections::execute_ssh_command))
        .route(
            "/terminate-connection/{conn_id}",
            post(connections::terminate),
        )
        .route(
            "/terminate-all-connections",
            post(connections::terminate_all),
        )
        .route("/active-connections", get(connections::active))
        .route("/disconnect", post(connections::disconnect_scoped))
        .route(
            "/preferences",
            get(prefs::preferences_get).post(prefs::preferences_set),
        )
        .route("/last-connection", get(prefs::last_connection))
        .route(
            "/instance-details/{instance_id}",
            get(aws::instance_details),
        )
        .route(
            "/windows-password/{instance_id}",
            post(aws::windows_password),
        )
        .route("/rdp-client/connect", post(connections::rdp_client_connect))
        .route("/health", get(aws::health))
        .route("/version", get(aws::version))
        .route("/check-update", get(aws::check_update))
        .route("/check-scp", get(transfer::check_scp))
        .route(
            "/transfer/{id}",
            post(transfer::start_transfer).delete(transfer::cancel_transfer),
        )
        .route(
            "/transfer-progress/{id}",
            get(transfer::get_transfer_progress),
        )
        .route("/quit", post(connections::quit))
        .layer(middleware::from_fn(require_api_token));

    let app = Router::new()
        .nest("/api", api)
        .nest("/static", static_router())
        .route("/", get(index_page))
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|err: BoxError| async move {
                    if err.is::<tower::timeout::error::Elapsed>() {
                        (axum::http::StatusCode::REQUEST_TIMEOUT, "Request timed out")
                    } else {
                        (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            "Unhandled error",
                        )
                    }
                }))
                .layer(TimeoutLayer::new(Duration::from_secs(30))),
        )
        .layer(ConcurrencyLimitLayer::new(256))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(cors)
        .layer(middleware::from_fn(check_host_header))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr: std::net::SocketAddr = "127.0.0.1:5000".parse().unwrap();
    let protocol = if use_https { "https" } else { "http" };
    let url = format!("{protocol}://{addr}");
    info!("Starting server on {url}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let tls_acceptor = if use_https {
        Some(TlsAcceptor::from(Arc::new(load_tls_config()?)))
    } else {
        None
    };
    let mut conn_builder = Builder::new(TokioExecutor::new());
    conn_builder
        .http1()
        .keep_alive(true)
        .auto_date_header(true)
        .title_case_headers(false)
        .timer(TokioTimer::new());
    conn_builder
        .http2()
        .adaptive_window(true)
        .max_concurrent_streams(Some(256))
        .keep_alive_interval(Some(Duration::from_secs(30)))
        .keep_alive_timeout(Duration::from_secs(10))
        .timer(TokioTimer::new());

    if let Err(err) = webbrowser::open(&url) {
        warn!("Failed to open browser: {err}");
    }

    let mut shutdown = std::pin::Pin::from(Box::new(tokio::signal::ctrl_c()));
    loop {
        tokio::select! {
            res = listener.accept() => {
                let (stream, _) = res?;
                let svc = TowerToHyperService::new(app.clone());
                let builder = conn_builder.clone();
                let tls_acceptor = tls_acceptor.clone();
                tokio::spawn(async move {
                    if let Some(tls_acceptor) = tls_acceptor {
                        match tls_acceptor.accept(stream).await {
                            Ok(s) => {
                                let io = TokioIo::new(s);
                                if let Err(err) = builder.serve_connection(io, svc).await {
                                    tracing::error!("HTTP connection error: {err}");
                                }
                            }
                            Err(err) => {
                                tracing::error!("TLS handshake error: {err}");
                            }
                        }
                    } else {
                        let io = TokioIo::new(stream);
                        if let Err(err) = builder.serve_connection(io, svc).await {
                            tracing::error!("HTTP connection error: {err}");
                        }
                    }
                });
            },
            _ = shutdown.as_mut() => {
                tracing::info!("Received shutdown signal (Ctrl+C)");
                break;
            }
            changed = shutdown_rx.changed() => {
                if changed.is_ok() && *shutdown_rx.borrow() {
                    tracing::info!("Received shutdown signal (API)");
                    break;
                }
            }
        }
    }

    cleanup_connections(&state).await;
    tracing::info!("Server shutdown complete");
    Ok(())
}

/// Loads TLS configuration for the server, including the certificate and private key. This function checks for the existence of the specified certificate and key files, and if they do not exist, it generates a new self-signed certificate with appropriate parameters (e.g. common name, subject alternative names, validity period) and saves them to disk in PEM format. It then loads the certificate and key from disk, constructs a rustls ServerConfig with the loaded credentials, and returns it. The function also configures ALPN protocols to support both HTTP/2 and HTTP/1.1. If any step in this process fails (e.g. file I/O errors, certificate generation errors), it returns an error with details about the failure.
/// Note: In a production environment, you would typically want to use a properly signed certificate from a trusted CA rather than a self-signed certificate, but for local development and testing purposes, this
/// function provides a convenient way to enable TLS support without additional setup steps. The generated self-signed certificate will be valid for 10 years and include "localhost" as a common name and subject alternative name, allowing it to work for local testing scenarios.
/// Args:
///   None. The function uses environment variables to determine the paths for the certificate and key files, with default values if the environment variables are not set.
/// Returns:
///   Ok(ServerConfig) if the TLS configuration was successfully loaded or generated, or an Err with details about the failure if there was an issue with file I/O, certificate generation, or configuration. The error message will include details about what went wrong to help diagnose issues with TLS setup.
/// Raises:
///   IOError: If there is an error reading the certificate or key files from disk (e.g. file not found, permission issues)
///   CertificateGenerationError: If there is an error generating the self-signed certificate (e.g. invalid parameters, cryptographic failure)  
fn load_tls_config() -> anyhow::Result<ServerConfig> {
    let cert_path = std::env::var("TLS_CERT_PATH").unwrap_or_else(|_| "certs/cert.pem".into());
    let key_path = std::env::var("TLS_KEY_PATH").unwrap_or_else(|_| "certs/key.pem".into());

    ensure_self_signed_cert(&cert_path, &key_path)?;

    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(&cert_path)?.collect::<Result<_, _>>()?;

    let key: PrivateKeyDer<'static> = PrivateKeyDer::from_pem_file(&key_path)
        .map_err(|e| anyhow::anyhow!("TLS private key error for {key_path}: {e}"))?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Ensures that a self-signed TLS certificate and private key exist at the specified paths. If the certificate or key files do not exist, this function generates a new self-signed certificate with appropriate parameters (e.g. common name, subject alternative names, validity period) and saves the certificate and key to disk in PEM format. If the files already exist, it does nothing. This allows the server to have TLS support out of the box without requiring manual certificate generation, while also ensuring that existing certificates are preserved if they are already in place.
/// Args:
///   cert_path: Path to the TLS certificate file (PEM format). If the file does not exist, a new self-signed certificate will be generated and saved to this path.
///   key_path: Path to the TLS private key file (PEM format). If the file does not exist, a new private key will be generated and saved to this path.
/// Returns:
///   Ok(()) if the certificate and key exist or were successfully generated and saved. Error if there was an issue generating the certificate or writing the files to disk (e.g. permission issues, invalid parameters).   
/// Raises:
///   IOError: If there is an error writing the certificate or key files to disk (e.g. permission issues, disk full)
///   CertificateGenerationError: If there is an error generating the self-signed certificate (e.g. invalid parameters, cryptographic failure)  
fn ensure_self_signed_cert(cert_path: &str, key_path: &str) -> anyhow::Result<()> {
    let cert_exists = std::fs::metadata(cert_path).is_ok();
    let key_exists = std::fs::metadata(key_path).is_ok();
    if cert_exists && key_exists {
        // Tighten permissions on keys created by earlier versions.
        restrict_key_permissions(key_path)?;
        return Ok(());
    }

    if let Some(parent) = std::path::Path::new(cert_path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = std::path::Path::new(key_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut params = CertificateParams::new(vec!["localhost".into()])?;
    params
        .subject_alt_names
        .push(SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::new(127, 0, 0, 1),
        )));
    params
        .subject_alt_names
        .push(SanType::IpAddress(std::net::IpAddr::V6(
            std::net::Ipv6Addr::LOCALHOST,
        )));
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "ssm-remote-session-gateway");
    params.distinguished_name = dn;

    let now = OffsetDateTime::now_utc();
    let not_before = now - time::Duration::days(1);
    let not_after = now + time::Duration::days(365 * 10);
    params.not_before = not_before;
    params.not_after = not_after;

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    std::fs::write(cert_path, cert.pem())?;
    write_private_key(key_path, &key_pair.serialize_pem())?;
    Ok(())
}

/// Writes the private key file, creating it with owner-only permissions on Unix.
/// Args:
///   path: Destination path of the PEM private key.
///   data: PEM-encoded private key contents.
/// Returns:
///   Ok(()) when the file is written with restricted permissions.
#[cfg(unix)]
fn write_private_key(path: &str, data: &str) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data.as_bytes())?;
    restrict_key_permissions(path)
}

/// Writes the private key file. Non-Unix platforms rely on default ACLs.
#[cfg(not(unix))]
fn write_private_key(path: &str, data: &str) -> std::io::Result<()> {
    std::fs::write(path, data)
}

/// Sets the private key file to owner read/write only (mode 0600) on Unix.
/// Args:
///   path: Path of the existing private key file.
/// Returns:
///   Ok(()) when permissions are applied or the platform does not support them.
#[cfg(unix)]
fn restrict_key_permissions(path: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
fn restrict_key_permissions(_path: &str) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_token_is_64_lowercase_hex_chars_and_unique() {
        let a = generate_api_token().expect("rng");
        let b = generate_api_token().expect("rng");
        assert_eq!(a.len(), 64);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(a, b);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_bytes() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret!"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
