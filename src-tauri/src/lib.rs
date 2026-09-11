//! The desktop shell.
//!
//! Command wrappers, state, and filesystem scope. **No logic** — every command here is a thin
//! call into [`rl_core::RouteLens`].
//!
//! That rule is what keeps discovery and the request engine testable with a plain
//! `cargo test`, with no GUI harness in the loop. If a command in this file starts making
//! decisions, the decision belongs in `rl-core` instead.

use rl_core::{ProjectScan, RouteLens, WorkspaceInfo};
use rl_http::Exchange;
use rl_model::RequestDraft;
use rl_workspace::{Collection, Environment, HistoryEntry, WorkspaceKind};
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;
use tokio::sync::Mutex;

/// Application state.
///
/// A `tokio` mutex rather than a `std` one because [`RouteLens::send`] is async and the guard
/// is held across an await point.
struct AppState {
    app: Mutex<RouteLens>,
}

/// An error on its way to the UI.
///
/// Carries the flattened source chain, because "request failed" on its own is not actionable
/// and "request failed: connection refused" is.
#[derive(Debug, Serialize)]
struct CommandError {
    message: String,
}

impl From<rl_core::CoreError> for CommandError {
    fn from(error: rl_core::CoreError) -> Self {
        CommandError {
            message: error.message(),
        }
    }
}

type CommandResult<T> = std::result::Result<T, CommandError>;

// --- workspace ---------------------------------------------------------------------------

#[tauri::command]
async fn open_workspace(state: State<'_, AppState>, path: PathBuf) -> CommandResult<WorkspaceInfo> {
    Ok(state.app.lock().await.open_workspace(path)?)
}

#[tauri::command]
async fn create_workspace(
    state: State<'_, AppState>,
    path: PathBuf,
    name: String,
    standalone: bool,
) -> CommandResult<WorkspaceInfo> {
    let kind = if standalone {
        WorkspaceKind::Standalone
    } else {
        WorkspaceKind::Project
    };
    Ok(state.app.lock().await.create_workspace(path, name, kind)?)
}

#[tauri::command]
async fn open_or_create_workspace(
    state: State<'_, AppState>,
    path: PathBuf,
    name: String,
) -> CommandResult<WorkspaceInfo> {
    Ok(state
        .app
        .lock()
        .await
        .open_or_create_workspace(path, name, WorkspaceKind::Project)?)
}

#[tauri::command]
async fn workspace_info(state: State<'_, AppState>) -> CommandResult<WorkspaceInfo> {
    Ok(state.app.lock().await.info()?)
}

#[tauri::command]
async fn close_workspace(state: State<'_, AppState>) -> CommandResult<()> {
    state.app.lock().await.close_workspace();
    Ok(())
}

// --- environments ------------------------------------------------------------------------

#[tauri::command]
async fn set_environment(
    state: State<'_, AppState>,
    name: Option<String>,
) -> CommandResult<WorkspaceInfo> {
    let mut app = state.app.lock().await;
    app.set_active_environment(name.as_deref())?;
    Ok(app.info()?)
}

#[tauri::command]
async fn delete_environment(
    state: State<'_, AppState>,
    name: String,
) -> CommandResult<WorkspaceInfo> {
    let mut app = state.app.lock().await;
    app.delete_environment(&name)?;
    Ok(app.info()?)
}

#[tauri::command]
async fn variable_names(state: State<'_, AppState>) -> CommandResult<Vec<String>> {
    Ok(state.app.lock().await.variable_names()?)
}

#[tauri::command]
async fn load_environment(state: State<'_, AppState>, name: String) -> CommandResult<Environment> {
    Ok(state.app.lock().await.load_environment(&name)?)
}

#[tauri::command]
async fn save_environment(
    state: State<'_, AppState>,
    environment: Environment,
) -> CommandResult<()> {
    Ok(state.app.lock().await.save_environment(&environment)?)
}

// --- secrets -----------------------------------------------------------------------------

/// Names only. Values are never sent to the UI — they are resolved in the engine, moments
/// before the request goes out.
#[tauri::command]
async fn secret_names(state: State<'_, AppState>) -> CommandResult<Vec<String>> {
    Ok(state.app.lock().await.secret_names()?)
}

#[tauri::command]
async fn set_secret(state: State<'_, AppState>, name: String, value: String) -> CommandResult<()> {
    Ok(state.app.lock().await.set_secret(&name, &value)?)
}

#[tauri::command]
async fn delete_secret(state: State<'_, AppState>, name: String) -> CommandResult<()> {
    Ok(state.app.lock().await.delete_secret(&name)?)
}

// --- collections -------------------------------------------------------------------------

#[tauri::command]
async fn load_collection(state: State<'_, AppState>, name: String) -> CommandResult<Collection> {
    Ok(state.app.lock().await.load_collection(&name)?)
}

#[tauri::command]
async fn save_request(
    state: State<'_, AppState>,
    collection: String,
    request: RequestDraft,
) -> CommandResult<()> {
    Ok(state.app.lock().await.save_request(&collection, request)?)
}

// --- discovery ---------------------------------------------------------------------------

/// Scan the open project for endpoints.
///
/// Static analysis only. Nothing in the project is executed — see `docs/security.md`.
#[tauri::command]
async fn scan_project(state: State<'_, AppState>) -> CommandResult<ProjectScan> {
    Ok(state.app.lock().await.scan()?)
}

#[tauri::command]
async fn open_endpoint(
    state: State<'_, AppState>,
    id: String,
    base_url: Option<String>,
) -> CommandResult<RequestDraft> {
    Ok(state
        .app
        .lock()
        .await
        .request_for(&id, base_url.as_deref())?)
}

/// Jump to where an endpoint is defined.
#[tauri::command]
async fn reveal_in_editor(
    state: State<'_, AppState>,
    file: PathBuf,
    line: u32,
) -> CommandResult<()> {
    Ok(state.app.lock().await.reveal_in_editor(&file, line)?)
}

// --- sending -----------------------------------------------------------------------------

#[tauri::command]
async fn send_request(
    state: State<'_, AppState>,
    request: RequestDraft,
) -> CommandResult<Exchange> {
    Ok(state.app.lock().await.send(&request).await?)
}

// --- history -----------------------------------------------------------------------------

#[tauri::command]
async fn history(state: State<'_, AppState>, limit: usize) -> CommandResult<Vec<HistoryEntry>> {
    Ok(state.app.lock().await.history(limit)?)
}

#[tauri::command]
async fn clear_history(state: State<'_, AppState>) -> CommandResult<()> {
    Ok(state.app.lock().await.clear_history()?)
}

// --- import ------------------------------------------------------------------------------

/// An import, with anything that could not be honoured.
#[derive(Debug, Serialize)]
struct ImportResult<T> {
    value: T,
    warnings: Vec<String>,
}

#[tauri::command]
async fn import_curl(
    state: State<'_, AppState>,
    text: String,
) -> CommandResult<ImportResult<RequestDraft>> {
    let imported = state.app.lock().await.import_curl(&text)?;
    Ok(ImportResult {
        value: imported.value,
        warnings: imported.warnings,
    })
}

#[tauri::command]
async fn import_raw_http(
    state: State<'_, AppState>,
    text: String,
) -> CommandResult<ImportResult<RequestDraft>> {
    let imported = state.app.lock().await.import_raw_http(&text)?;
    Ok(ImportResult {
        value: imported.value,
        warnings: imported.warnings,
    })
}

/// Summary of an imported specification. The endpoints themselves land in a collection.
#[derive(Debug, Serialize)]
struct OpenApiSummary {
    title: String,
    version: String,
    servers: Vec<String>,
    endpoint_count: usize,
}

#[tauri::command]
async fn import_openapi(
    state: State<'_, AppState>,
    text: String,
    collection: String,
) -> CommandResult<ImportResult<OpenApiSummary>> {
    let imported = state
        .app
        .lock()
        .await
        .import_openapi_as_collection(&text, &collection)?;

    Ok(ImportResult {
        value: OpenApiSummary {
            title: imported.value.title,
            version: imported.value.version,
            servers: imported.value.servers,
            endpoint_count: imported.value.endpoints.len(),
        },
        warnings: imported.warnings,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            app: Mutex::new(RouteLens::new()),
        })
        .invoke_handler(tauri::generate_handler![
            open_workspace,
            create_workspace,
            open_or_create_workspace,
            workspace_info,
            close_workspace,
            set_environment,
            load_environment,
            save_environment,
            secret_names,
            set_secret,
            delete_secret,
            delete_environment,
            variable_names,
            load_collection,
            save_request,
            scan_project,
            open_endpoint,
            reveal_in_editor,
            send_request,
            history,
            clear_history,
            import_curl,
            import_raw_http,
            import_openapi,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RouteLens");
}
