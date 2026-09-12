//! Loopback-only HTTP API and embedded browser UI for biomedical input optimization.

mod acquisition;
mod benchmark;
mod error;
mod github;
mod models;
mod pipeline;
mod workflow;

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::{
        HeaderMap, HeaderValue, Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, HOST, ORIGIN},
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use qbm_app::PlatformApp;
use qbm_domain::RunId;
use serde::Serialize;
use thiserror::Error;
use tokio::{net::TcpListener, sync::Mutex};

use crate::{
    acquisition::{acquire_archive, acquire_folder, acquire_github},
    error::ApiError,
    github::GithubClient,
    models::{BootstrapView, DecisionRequest, GithubImportRequest},
    workflow::{
        advance_run, approved_optimization_export, approved_profile_export,
        approved_quantum_export, approved_wiring_diagnostic_export, decide_and_advance,
        finish_import, recent_runs, report_bundle, workflow_view,
    },
};

const DEFAULT_MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_MAX_FOLDER_FILES: usize = 20_000;
const DEFAULT_MAX_FOLDER_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MULTIPART_OVERHEAD_BYTES: usize = 16 * 1024 * 1024;

/// Configurable local HTTP safety limits.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Maximum compressed archive or aggregate folder-upload bytes.
    pub max_upload_bytes: u64,
    /// Maximum files submitted by browser folder selection.
    pub max_folder_files: usize,
    /// Maximum bytes for one browser-uploaded folder file.
    pub max_folder_file_bytes: u64,
    /// Open the loopback URL in the operating system's default browser.
    pub open_browser: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            max_folder_files: DEFAULT_MAX_FOLDER_FILES,
            max_folder_file_bytes: DEFAULT_MAX_FOLDER_FILE_BYTES,
            open_browser: false,
        }
    }
}

/// Browser-service startup or listener failure.
#[derive(Debug, Error)]
pub enum WebServerError {
    /// Platform data directory could not be opened.
    #[error(transparent)]
    App(#[from] qbm_app::AppError),
    /// Loopback listener or HTTP service failed.
    #[error("local browser service failed: {0}")]
    Io(#[from] std::io::Error),
    /// Internal browser-service state could not be initialized.
    #[error("local browser service initialization failed: {0}")]
    Initialization(String),
}

#[derive(Debug, Clone)]
struct AppState {
    app: PlatformApp,
    data_directory: PathBuf,
    csrf_token: String,
    origin: String,
    authority: String,
    config: WebConfig,
    github: GithubClient,
    import_gate: Arc<Mutex<()>>,
}

#[derive(Debug, Serialize)]
struct HealthView {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

/// Run the local browser service on `127.0.0.1` until interrupted.
pub async fn serve(
    data_directory: impl AsRef<Path>,
    port: u16,
    config: WebConfig,
) -> Result<(), WebServerError> {
    let listener =
        TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).await?;
    let address = listener.local_addr()?;
    let app = PlatformApp::open(data_directory.as_ref())?;
    let open_browser = config.open_browser;
    let router = build_router(app, data_directory.as_ref(), address, config)
        .map_err(|error| WebServerError::Initialization(format!("{error:?}")))?;
    println!("Q-BenchMed is ready at http://{address}");
    println!("Source code is read as data and never executed. Press Ctrl+C to stop.");
    if open_browser {
        open_local_browser(&format!("http://{address}"));
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn build_router(
    app: PlatformApp,
    data_directory: &Path,
    address: SocketAddr,
    config: WebConfig,
) -> Result<Router, ApiError> {
    let state = AppState {
        app,
        data_directory: data_directory.to_path_buf(),
        csrf_token: uuid::Uuid::new_v4().to_string(),
        origin: format!("http://{address}"),
        authority: address.to_string(),
        config: config.clone(),
        github: GithubClient::new()?,
        import_gate: Arc::new(Mutex::new(())),
    };
    let body_limit = usize::try_from(config.max_upload_bytes)
        .unwrap_or(usize::MAX - MULTIPART_OVERHEAD_BYTES)
        .saturating_add(MULTIPART_OVERHEAD_BYTES);
    Ok(Router::new()
        .route("/", get(index))
        .route("/assets/app.css", get(styles))
        .route("/assets/app.js", get(script))
        .route("/health", get(health))
        .route("/api/bootstrap", get(bootstrap))
        .route("/api/import/archive", post(import_archive))
        .route("/api/import/folder", post(import_folder))
        .route("/api/import/github", post(import_github))
        .route("/api/runs/{run_id}", get(get_workflow))
        .route("/api/runs/{run_id}/decision", post(post_decision))
        .route("/api/runs/{run_id}/advance", post(post_advance))
        .route("/api/runs/{run_id}/report", get(download_report))
        .route("/api/runs/{run_id}/profile", get(download_profile))
        .route(
            "/api/runs/{run_id}/optimization",
            get(download_optimization),
        )
        .route(
            "/api/runs/{run_id}/wiring-diagnostic",
            get(download_wiring_diagnostic),
        )
        .route(
            "/api/runs/{run_id}/quantum-formulation",
            get(download_quantum_formulation),
        )
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_boundary,
        ))
        .with_state(state))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../assets/index.html"))
}

async fn styles() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../assets/app.css"),
    )
}

async fn script() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../assets/app.js"),
    )
}

async fn health() -> Json<HealthView> {
    Json(HealthView {
        status: "ok",
        service: "qbenchmed-platform",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn bootstrap(State(state): State<AppState>) -> Result<Json<BootstrapView>, ApiError> {
    Ok(Json(BootstrapView {
        schema_version: "qbm.web-bootstrap/v1",
        csrf_token: state.csrf_token,
        service: "Q-BenchMed Local",
        version: env!("CARGO_PKG_VERSION"),
        source_types: ["uploaded_archive", "uploaded_folder", "public_github"],
        max_upload_bytes: state.config.max_upload_bytes,
        recent_runs: recent_runs(&state.app)?,
    }))
}

async fn import_archive(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<models::WorkflowView>, ApiError> {
    verify_mutation(&state, &headers)?;
    let _guard = state.import_gate.lock().await;
    let (source, requested_name) = acquire_archive(
        &state.data_directory,
        multipart,
        state.config.max_upload_bytes,
    )
    .await?;
    let app = state.app.clone();
    let workflow = tokio::task::spawn_blocking(move || finish_import(&app, source, requested_name))
        .await
        .map_err(|_| ApiError::internal("The archive audit task stopped unexpectedly."))??;
    Ok(Json(workflow))
}

async fn import_folder(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<models::WorkflowView>, ApiError> {
    verify_mutation(&state, &headers)?;
    let _guard = state.import_gate.lock().await;
    let (source, requested_name) = acquire_folder(
        &state.data_directory,
        multipart,
        state.config.max_upload_bytes,
        state.config.max_folder_files,
        state.config.max_folder_file_bytes,
    )
    .await?;
    let app = state.app.clone();
    let workflow = tokio::task::spawn_blocking(move || finish_import(&app, source, requested_name))
        .await
        .map_err(|_| ApiError::internal("The folder audit task stopped unexpectedly."))??;
    Ok(Json(workflow))
}

async fn import_github(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<GithubImportRequest>,
) -> Result<Json<models::WorkflowView>, ApiError> {
    verify_mutation(&state, &headers)?;
    let _guard = state.import_gate.lock().await;
    let source = acquire_github(
        &state.data_directory,
        &state.github,
        &request.repository_url,
        request.reference.as_deref(),
        state.config.max_upload_bytes,
    )
    .await?;
    let app = state.app.clone();
    let workflow =
        tokio::task::spawn_blocking(move || finish_import(&app, source, request.project_name))
            .await
            .map_err(|_| ApiError::internal("The GitHub audit task stopped unexpectedly."))??;
    Ok(Json(workflow))
}

async fn get_workflow(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Json<models::WorkflowView>, ApiError> {
    Ok(Json(workflow_view(&state.app, parse_run_id(&run_id)?)?))
}

async fn post_decision(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<DecisionRequest>,
) -> Result<Json<models::WorkflowView>, ApiError> {
    verify_mutation(&state, &headers)?;
    let run_id = parse_run_id(&run_id)?;
    let app = state.app.clone();
    let workflow = tokio::task::spawn_blocking(move || {
        decide_and_advance(
            &app,
            run_id,
            &request.stage_id,
            &request.expected_output_hash,
            request.decision,
            &request.actor_id,
            request.reason,
        )
    })
    .await
    .map_err(|_| ApiError::internal("The approval task stopped unexpectedly."))??;
    Ok(Json(workflow))
}

async fn post_advance(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<models::WorkflowView>, ApiError> {
    verify_mutation(&state, &headers)?;
    let run_id = parse_run_id(&run_id)?;
    let app = state.app.clone();
    tokio::task::spawn_blocking(move || advance_run(&app, run_id))
        .await
        .map_err(|_| ApiError::internal("The audit task stopped unexpectedly."))??;
    Ok(Json(workflow_view(&state.app, run_id)?))
}

async fn download_report(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let bundle = report_bundle(&state.app, run_id)?;
    let bytes = serde_json::to_vec_pretty(&bundle)
        .map_err(|_| ApiError::internal("The report bundle could not be serialized."))?;
    Ok((
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!(
                    "attachment; filename=\"qbenchmed-audit-{run_id}.json\""
                ))
                .map_err(|_| ApiError::internal("The report filename was invalid."))?,
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn download_profile(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let profile = approved_profile_export(&state.app, run_id)?;
    json_download(
        &profile,
        format!("qbenchmed-profile-{run_id}.json"),
        "The approved qbm.profile could not be serialized.",
    )
}

async fn download_optimization(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let export = approved_optimization_export(&state.app, run_id)?;
    json_download(
        &export,
        format!("qbenchmed-optimization-{run_id}.json"),
        "The approved classical optimization analysis could not be serialized.",
    )
}

async fn download_wiring_diagnostic(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let export = approved_wiring_diagnostic_export(&state.app, run_id)?;
    json_download(
        &export,
        format!("qbenchmed-wiring-diagnostic-{run_id}.json"),
        "The approved profile wiring diagnostic could not be serialized.",
    )
}

async fn download_quantum_formulation(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let export = approved_quantum_export(&state.app, run_id)?;
    json_download(
        &export,
        format!("qbenchmed-quantum-formulation-{run_id}.json"),
        "The approved QUBO/Ising formulation could not be serialized.",
    )
}

fn json_download<T: Serialize>(
    value: &T,
    filename: String,
    serialization_error: &'static str,
) -> Result<Response, ApiError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|_| ApiError::internal(serialization_error))?;
    Ok((
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                    .map_err(|_| ApiError::internal("The download filename was invalid."))?,
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn not_found() -> ApiError {
    ApiError::not_found("not_found", "The requested local resource does not exist.")
}

async fn security_boundary(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok());
    if host != Some(state.authority.as_str()) {
        return Err(ApiError::forbidden(
            "host_refused",
            "The local service refused an unexpected Host header.",
        ));
    }
    let is_api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    );
    if is_api {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    Ok(response)
}

fn verify_mutation(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get(ORIGIN).and_then(|value| value.to_str().ok()) != Some(state.origin.as_str()) {
        return Err(ApiError::forbidden(
            "origin_refused",
            "This change must come from the Q-BenchMed page on this computer.",
        ));
    }
    if headers
        .get("Sec-Fetch-Site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value != "same-origin" && value != "none")
    {
        return Err(ApiError::forbidden(
            "cross_site_refused",
            "Cross-site requests are not allowed.",
        ));
    }
    if headers
        .get("X-QBM-CSRF")
        .and_then(|value| value.to_str().ok())
        != Some(state.csrf_token.as_str())
    {
        return Err(ApiError::forbidden(
            "csrf_refused",
            "The local page security token is missing or expired. Reload the page.",
        ));
    }
    Ok(())
}

fn parse_run_id(value: &str) -> Result<RunId, ApiError> {
    RunId::from_str(value).map_err(|_| {
        ApiError::bad_request("invalid_run_id", "The audit run identifier is invalid.")
    })
}

fn open_local_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if result.is_err() {
        eprintln!("Could not open a browser automatically. Open {url} manually.");
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tower::ServiceExt;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;
    use crate::models::WORKFLOW_STAGES;

    fn test_router(directory: &TempDir) -> Router {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 43191);
        build_router(
            PlatformApp::open(directory.path()).unwrap(),
            directory.path(),
            address,
            WebConfig {
                max_upload_bytes: 1024 * 1024,
                max_folder_files: 100,
                max_folder_file_bytes: 512 * 1024,
                open_browser: false,
            },
        )
        .unwrap()
    }

    async fn bootstrap_token(router: &Router) -> String {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/bootstrap")
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["csrf_token"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn zip_bytes(path: &str, content: &[u8]) -> Vec<u8> {
        let output = Cursor::new(Vec::new());
        let mut archive = ZipWriter::new(output);
        archive
            .start_file(path, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(content).unwrap();
        archive.finish().unwrap().into_inner()
    }

    fn archive_multipart(archive: &[u8]) -> (String, Vec<u8>) {
        let boundary = "qbm-browser-test-boundary";
        let mut body = Vec::new();
        write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"project_name\"\r\n\r\nWeb fixture\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"source\"; filename=\"fixture.zip\"\r\nContent-Type: application/zip\r\n\r\n"
        )
        .unwrap();
        body.extend_from_slice(archive);
        write!(body, "\r\n--{boundary}--\r\n").unwrap();
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    async fn response_json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn approve_to_completion(router: &Router, csrf: &str, mut workflow: Value) -> Value {
        let run_id = workflow["run_id"].as_str().unwrap().to_owned();
        for _ in 0..WORKFLOW_STAGES.len() {
            if workflow["run_state"] == "complete" {
                break;
            }
            let stage = workflow["stages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|stage| stage["status"] == "waiting_approval")
                .unwrap();
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/runs/{run_id}/decision"))
                        .header(HOST, "127.0.0.1:43191")
                        .header(ORIGIN, "http://127.0.0.1:43191")
                        .header("X-QBM-CSRF", csrf)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            serde_json::to_vec(&json!({
                                "stage_id": stage["id"],
                                "expected_output_hash": stage["output"]["output_hash"],
                                "decision": "approve",
                                "actor_id": "browser-test",
                                "reason": null
                            }))
                            .unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            workflow = response_json(response).await;
        }
        workflow
    }

    #[tokio::test]
    async fn host_guard_and_security_headers_protect_the_ui() {
        let directory = TempDir::new().unwrap();
        let router = test_router(&directory);
        let refused = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(HOST, "attacker.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        let accepted = router
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(accepted.headers().get("X-Frame-Options").unwrap(), "DENY");
        assert!(accepted.headers().contains_key("Content-Security-Policy"));
    }

    #[tokio::test]
    async fn mutation_requires_same_origin_and_csrf_token() {
        let directory = TempDir::new().unwrap();
        let router = test_router(&directory);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/import/github")
                    .header(HOST, "127.0.0.1:43191")
                    .header(ORIGIN, "https://evil.example")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"repository_url":"https://github.com/openai/openai-rust"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn zip_upload_completes_full_governed_report_with_honest_skips() {
        let directory = TempDir::new().unwrap();
        let router = test_router(&directory);
        let csrf = bootstrap_token(&router).await;
        let archive = zip_bytes(
            "rules.yaml",
            b"definitions:\n  one:\n    id: same\n  two:\n    id: same\nflow:\n  ref: missing\n",
        );
        let (content_type, body) = archive_multipart(&archive);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/import/archive")
                    .header(HOST, "127.0.0.1:43191")
                    .header(ORIGIN, "http://127.0.0.1:43191")
                    .header("X-QBM-CSRF", &csrf)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let workflow = response_json(response).await;
        let run_id = workflow["run_id"].as_str().unwrap().to_owned();
        let workflow = approve_to_completion(&router, &csrf, workflow).await;

        assert_eq!(workflow["run_state"], "complete");
        assert_eq!(workflow["event_chain_valid"], true);
        assert!(workflow["report"]["summary"]["errors"].as_u64().unwrap() >= 2);
        assert_eq!(workflow["biomedical_profile"]["status"], "skipped");
        assert_eq!(workflow["classical_optimization"]["status"], "skipped");
        assert_eq!(workflow["qubo_ising_validation"]["status"], "skipped");
        assert!(workflow["profile_download_url"].is_null());
        assert!(workflow["quantum_export_url"].is_null());
        assert_eq!(
            workflow["full_report"]["schema_version"],
            "qbm.full-report/v1"
        );
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/runs/{run_id}/report"))
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert!(response.headers().contains_key(CONTENT_DISPOSITION));
        let bundle = response_json(response).await;
        assert_eq!(bundle["schema_version"], "qbm.audit-bundle/v2");
        assert_eq!(
            bundle["full_report"]["schema_version"],
            "qbm.full-report/v1"
        );
        assert_eq!(
            bundle["events"].as_array().unwrap().last().unwrap()["kind"],
            "run_completed"
        );
        assert!(bundle["source"].get("managed_path").is_none());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn approved_profile_optimization_wiring_and_quantum_exports_are_downloadable() {
        let directory = TempDir::new().unwrap();
        let router = test_router(&directory);
        let csrf = bootstrap_token(&router).await;
        let profile = json!({
            "schema_version": "qbm.benchmark-profile/v1",
            "profile_id": "biomedical/test-panel",
            "title": "Test biomarker panel",
            "biomedical_scope": {
                "area": "oncology",
                "population": "synthetic validation cohort",
                "input_semantics": "candidate biomarker assay",
                "outcome_semantics": "covered molecular finding"
            },
            "inputs": [
                {"id": "assay_a", "label": "Assay A", "cost": 1.0, "tags": ["synthetic"]},
                {"id": "assay_b", "label": "Assay B", "cost": 2.0, "tags": ["synthetic"]},
                {"id": "assay_c", "label": "Assay C", "cost": 3.0, "tags": ["synthetic"]}
            ],
            "outcomes": [
                {"id": "finding_a", "label": "Finding A", "weight": 1.0, "tags": ["synthetic"]},
                {"id": "finding_b", "label": "Finding B", "weight": 2.0, "tags": ["synthetic"]},
                {"id": "finding_c", "label": "Finding C", "weight": 3.0, "tags": ["synthetic"]}
            ],
            "relationships": [
                {"input_id": "assay_a", "outcome_id": "finding_a"},
                {"input_id": "assay_b", "outcome_id": "finding_b"}
            ],
            "constraints": {
                "min_selected": 0,
                "max_selected": 1,
                "max_total_cost": null,
                "required_inputs": [],
                "excluded_inputs": [],
                "required_outcomes": []
            },
            "objective": "maximize_weighted_coverage",
            "provenance": {
                "generated_by": "browser-test",
                "source_revision": "fixture-v1",
                "source_artifact_ids": [],
                "projection_method": "explicit test profile"
            }
        });
        let archive = zip_bytes("qbm.profile.json", &serde_json::to_vec(&profile).unwrap());
        let (content_type, body) = archive_multipart(&archive);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/import/archive")
                    .header(HOST, "127.0.0.1:43191")
                    .header(ORIGIN, "http://127.0.0.1:43191")
                    .header("X-QBM-CSRF", &csrf)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let initial_workflow = response_json(response).await;
        let initial_run_id = initial_workflow["run_id"].as_str().unwrap();
        for path in ["optimization", "wiring-diagnostic"] {
            let unavailable = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/runs/{initial_run_id}/{path}"))
                        .header(HOST, "127.0.0.1:43191")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(unavailable.status(), StatusCode::CONFLICT);
        }
        let workflow = approve_to_completion(&router, &csrf, initial_workflow).await;
        let run_id = workflow["run_id"].as_str().unwrap();
        assert_eq!(workflow["run_state"], "complete");
        assert_eq!(
            workflow["profile_download_url"],
            format!("/api/runs/{run_id}/profile")
        );
        assert_eq!(
            workflow["optimization_download_url"],
            format!("/api/runs/{run_id}/optimization")
        );
        assert_eq!(
            workflow["wiring_diagnostic_download_url"],
            format!("/api/runs/{run_id}/wiring-diagnostic")
        );
        assert_eq!(
            workflow["quantum_export_url"],
            format!("/api/runs/{run_id}/quantum-formulation")
        );

        let profile_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/runs/{run_id}/profile"))
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(profile_response.status(), StatusCode::OK);
        let exported_profile = response_json(profile_response).await;
        assert_eq!(
            exported_profile["schema_version"],
            "qbm.benchmark-profile/v1"
        );
        assert_eq!(exported_profile["inputs"].as_array().unwrap().len(), 3);

        let optimization_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/runs/{run_id}/optimization"))
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(optimization_response.status(), StatusCode::OK);
        let optimization = response_json(optimization_response).await;
        assert_eq!(
            optimization["schema_version"],
            "qbm.classical-optimization-export/v1"
        );
        assert_eq!(
            optimization["optimization"]["structural"]["inert_inputs"],
            json!(["assay_c"])
        );
        assert_eq!(
            optimization["optimization"]["structural"]["unreachable_outcomes"],
            json!(["finding_c"])
        );
        let coverage = optimization["optimization"]["coverage_at_k"]
            .as_array()
            .unwrap();
        assert!(!coverage.is_empty());
        assert!(coverage[0]["result"]["score"]["selected_inputs"].is_array());
        assert!(coverage[0]["result"]["score"]["covered_outcomes"].is_array());
        assert!(coverage[0]["result"]["score"]["uncovered_outcomes"].is_array());

        let wiring_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/runs/{run_id}/wiring-diagnostic"))
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wiring_response.status(), StatusCode::OK);
        let wiring = response_json(wiring_response).await;
        assert_eq!(wiring["schema_version"], "qbm.wiring-diagnostic-export/v1");
        assert_eq!(wiring["summary"]["input_count"], 3);
        assert_eq!(wiring["summary"]["wired_input_count"], 2);
        assert_eq!(wiring["summary"]["inert_input_count"], 1);
        assert_eq!(wiring["summary"]["reachable_outcome_count"], 2);
        assert_eq!(wiring["summary"]["unreachable_outcome_count"], 1);
        assert_eq!(wiring["relationships"].as_array().unwrap().len(), 2);
        let inert = wiring["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["id"] == "assay_c")
            .unwrap();
        assert_eq!(inert["status"], "inert");
        assert!(
            inert["reason"]
                .as_str()
                .unwrap()
                .contains("approved qbm.profile")
        );
        assert!(!inert["evidence_refs"].as_array().unwrap().is_empty());
        let unreachable = wiring["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outcome| outcome["id"] == "finding_c")
            .unwrap();
        assert_eq!(unreachable["status"], "unreachable");
        assert!(
            wiring["interpretation_boundary"]
                .as_str()
                .unwrap()
                .contains("not proof")
        );
        assert_eq!(wiring["projection"]["method"], "explicit_profile");
        assert_eq!(wiring["projection"]["confidence_bps"], 9900);
        assert!(
            !wiring["projection"]["evidence_catalog"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(wiring["semantic_layer"]["status"], "not_applicable");

        let quantum_response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/runs/{run_id}/quantum-formulation"))
                    .header(HOST, "127.0.0.1:43191")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(quantum_response.status(), StatusCode::OK);
        let export = response_json(quantum_response).await;
        assert_eq!(
            export["schema_version"],
            "qbm.quantum-formulation-export/v1"
        );
        assert!(
            export["formulation"]["energy_validation"]["assignments_checked"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(
            export["formulation"]["energy_validation"]["maximum_absolute_error"]
                .as_f64()
                .unwrap()
                <= f64::EPSILON
        );
        assert_eq!(export["formulation"]["execution"]["status"], "export_only");
    }

    #[tokio::test]
    async fn malicious_zip_path_is_rejected_without_extraction() {
        let directory = TempDir::new().unwrap();
        let router = test_router(&directory);
        let csrf = bootstrap_token(&router).await;
        let archive = zip_bytes("../../outside.txt", b"must never be extracted");
        let (content_type, body) = archive_multipart(&archive);
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/import/archive")
                    .header(HOST, "127.0.0.1:43191")
                    .header(ORIGIN, "http://127.0.0.1:43191")
                    .header("X-QBM-CSRF", &csrf)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!directory.path().join("outside.txt").exists());
    }
}
