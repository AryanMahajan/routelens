//! # rl-core
//!
//! The facade. Owns application state, orchestrates the other crates, and exposes one
//! coherent API.
//!
//! ```text
//! rl-model  ←──  rl-discovery
//!     ↑     ←──  rl-import
//!     │     ←──  rl-http
//!     │     ←──  rl-workspace
//!     │               │
//!     └────  rl-core ─┘
//!                │
//!           src-tauri      ← command wrappers only, no logic
//!                │
//!               ui
//! ```
//!
//! ## The boundary this crate exists to hold
//!
//! `src-tauri` contains **no logic** — only command wrappers, event emission, and filesystem
//! scope handling. Everything it calls lives here.
//!
//! That keeps discovery testable with a plain `cargo test` against fixture repositories, with
//! no GUI harness in the loop, which matters because discovery is the highest-risk component.
//! It also means a second shell — a CLI, for use over SSH or in a dev container — would be a
//! small binary over an existing library rather than a rewrite.
//!
//! ## The send path
//!
//! [`RouteLens::send`] is where the crates meet, and the order matters:
//!
//! 1. Assemble variables from the workspace's three tiers.
//! 2. Resolve `{{variables}}` — as late as possible, so a plaintext secret exists briefly.
//! 3. Execute.
//! 4. Record to history, redacted.
//!
//! Step 4 cannot be skipped by accident: [`rl_workspace::History::record`] accepts only a
//! `RedactedEntry`.

#![forbid(unsafe_code)]

pub mod error;

pub use error::{CoreError, Result};

use rl_discovery::ScanResult;
use rl_http::{Exchange, HttpEngine};
use rl_import::{Imported, OpenApiImport};
use rl_model::{RequestDraft, VariableContext};
use rl_workspace::{
    Collection, Environment, History, HistoryEntry, NewEntry, SecretStore, Workspace, WorkspaceKind,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A summary of the open workspace, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
    pub root: PathBuf,
    pub kind: WorkspaceKind,
    pub collections: Vec<String>,
    pub environments: Vec<String>,
    pub active_environment: Option<String>,
    /// Secret names the active environment expects but the store does not hold.
    pub missing_secrets: Vec<String>,
}

/// One discovered endpoint, flattened for the UI.
///
/// A deliberate DTO rather than sending [`rl_model::EndpointSpec`] straight out: the model's
/// path is a segment list, and the UI wants a rendered string plus the handful of booleans it
/// actually branches on. Keeping the wire shape explicit means changing the model does not
/// silently change what the UI receives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointView {
    pub id: String,
    pub method: String,
    /// Rendered in `{brace}` form.
    pub path: String,
    /// `GET /api/v1/users/{user_id}`
    pub display: String,
    pub group: Option<String>,
    pub summary: Option<String>,
    pub source: Option<SourceView>,
    /// The router this belongs to is never mounted.
    pub orphaned: bool,
    /// At least one path segment could not be determined statically.
    pub unresolved: bool,
    /// The source expressions that defeated resolution, for the tooltip.
    pub unresolved_exprs: Vec<String>,
    pub auth: bool,
    pub has_body: bool,
    pub query: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceView {
    /// Forward slashes, so the UI renders identically on every platform.
    pub file: String,
    pub line: u32,
}

/// What a scan hands the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectScan {
    pub frameworks: Vec<rl_discovery::DetectedFramework>,
    pub endpoints: Vec<EndpointView>,
    pub base_urls: Vec<rl_discovery::BaseUrlCandidate>,
    pub warnings: Vec<String>,
    pub stats: rl_discovery::ScanStats,
}

impl ProjectScan {
    fn from_result(result: &ScanResult) -> ProjectScan {
        ProjectScan {
            frameworks: result.frameworks.clone(),
            endpoints: result.endpoints.iter().map(to_view).collect(),
            base_urls: result.base_urls.clone(),
            warnings: result.warnings.clone(),
            stats: result.stats,
        }
    }
}

fn to_view(spec: &rl_model::EndpointSpec) -> EndpointView {
    EndpointView {
        id: spec.id.as_str().to_string(),
        method: spec.method.to_string(),
        path: spec.path.render(rl_model::ParamStyle::Braces),
        display: spec.display(),
        group: spec.group.clone(),
        summary: spec.summary.clone(),
        source: spec.source.as_ref().map(|s| SourceView {
            file: s.file.display().to_string().replace('\\', "/"),
            line: s.line,
        }),
        orphaned: spec.orphaned,
        unresolved: !spec.path.is_resolved(),
        unresolved_exprs: spec
            .path
            .unresolved_exprs()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        auth: spec.auth.is_some(),
        has_body: spec.body.is_some(),
        query: spec.query_params.iter().map(|p| p.name.clone()).collect(),
    }
}

/// The application.
#[derive(Debug, Default)]
pub struct RouteLens {
    workspace: Option<Workspace>,
    active_environment: Option<String>,
    engine: HttpEngine,
    /// Kept so the UI can open an endpoint by id without rescanning.
    last_scan: Option<ScanResult>,
}

impl RouteLens {
    pub fn new() -> Self {
        Self::default()
    }

    // --- workspace ----------------------------------------------------------------------

    pub fn open_workspace(&mut self, root: impl AsRef<Path>) -> Result<WorkspaceInfo> {
        let workspace = Workspace::open(root)?;
        self.active_environment = workspace.manifest().default_environment.clone();
        self.workspace = Some(workspace);
        self.last_scan = None;
        self.info()
    }

    pub fn create_workspace(
        &mut self,
        root: impl AsRef<Path>,
        name: impl Into<String>,
        kind: WorkspaceKind,
    ) -> Result<WorkspaceInfo> {
        let workspace = Workspace::create(root, name, kind)?;
        self.active_environment = None;
        self.workspace = Some(workspace);
        self.info()
    }

    pub fn open_or_create_workspace(
        &mut self,
        root: impl AsRef<Path>,
        name: impl Into<String>,
        kind: WorkspaceKind,
    ) -> Result<WorkspaceInfo> {
        let root = root.as_ref();
        if Workspace::exists(root) {
            self.open_workspace(root)
        } else {
            self.create_workspace(root, name, kind)
        }
    }

    pub fn close_workspace(&mut self) {
        self.workspace = None;
        self.active_environment = None;
        self.last_scan = None;
    }

    pub fn is_open(&self) -> bool {
        self.workspace.is_some()
    }

    fn workspace(&self) -> Result<&Workspace> {
        self.workspace.as_ref().ok_or(CoreError::NoWorkspace)
    }

    pub fn info(&self) -> Result<WorkspaceInfo> {
        let workspace = self.workspace()?;
        let manifest = workspace.manifest();

        let missing_secrets = match &self.active_environment {
            Some(name) => match workspace.load_environment(name) {
                Ok(env) => env.missing_secrets(&workspace.secrets().load_all()?),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };

        Ok(WorkspaceInfo {
            name: manifest.name.clone(),
            root: workspace.layout().root().to_path_buf(),
            kind: manifest.kind,
            collections: workspace.collection_names()?,
            environments: workspace.environment_names()?,
            active_environment: self.active_environment.clone(),
            missing_secrets,
        })
    }

    // --- environments -------------------------------------------------------------------

    pub fn active_environment(&self) -> Option<&str> {
        self.active_environment.as_deref()
    }

    /// Switch environments, refusing a name that does not exist rather than silently
    /// falling back to no variables at all.
    pub fn set_active_environment(&mut self, name: Option<&str>) -> Result<()> {
        if let Some(name) = name {
            self.workspace()?.load_environment(name)?;
        }
        self.active_environment = name.map(str::to_string);
        Ok(())
    }

    pub fn load_environment(&self, name: &str) -> Result<Environment> {
        Ok(self.workspace()?.load_environment(name)?)
    }

    pub fn save_environment(&self, environment: &Environment) -> Result<()> {
        Ok(self.workspace()?.save_environment(environment)?)
    }

    pub fn variables(&self) -> Result<VariableContext> {
        Ok(self
            .workspace()?
            .variable_context(self.active_environment.as_deref())?)
    }

    // --- secrets ------------------------------------------------------------------------

    pub fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        self.workspace()?.secrets().set(name, value)?;
        Ok(())
    }

    pub fn delete_secret(&self, name: &str) -> Result<()> {
        self.workspace()?.secrets().delete(name)?;
        Ok(())
    }

    /// Names only. Values never leave this process for the UI.
    pub fn secret_names(&self) -> Result<Vec<String>> {
        Ok(self.workspace()?.secrets().names()?)
    }

    // --- collections --------------------------------------------------------------------

    pub fn collection_names(&self) -> Result<Vec<String>> {
        Ok(self.workspace()?.collection_names()?)
    }

    pub fn load_collection(&self, name: &str) -> Result<Collection> {
        Ok(self.workspace()?.load_collection(name)?)
    }

    pub fn save_collection(&self, collection: &Collection) -> Result<()> {
        Ok(self.workspace()?.save_collection(collection)?)
    }

    /// Append a request to a collection, creating it if needed.
    pub fn save_request(&self, collection_name: &str, request: RequestDraft) -> Result<()> {
        let workspace = self.workspace()?;
        let mut collection = workspace
            .load_collection(collection_name)
            .unwrap_or_else(|_| Collection::new(collection_name));

        // Replace by name when one already exists, so saving twice does not duplicate.
        match request.name.as_deref().and_then(|name| {
            collection
                .requests
                .iter()
                .position(|r| r.name.as_deref() == Some(name))
        }) {
            Some(index) => collection.requests[index] = request,
            None => collection.push(request),
        }

        workspace.save_collection(&collection)?;
        Ok(())
    }

    // --- discovery ----------------------------------------------------------------------

    /// Scan the open workspace's project for endpoints.
    ///
    /// Static analysis only — nothing in the project is executed. See `docs/security.md`.
    pub fn scan(&mut self) -> Result<ProjectScan> {
        let workspace = self.workspace()?;
        if workspace.manifest().kind == WorkspaceKind::Standalone {
            return Err(CoreError::NotAProject);
        }

        let root = workspace.layout().root().to_path_buf();
        let result = rl_discovery::scan(&root)?;

        // Seed the environment's base URL from the best candidate, but never overwrite one
        // the developer has already chosen.
        if let (Some(name), Some(candidate)) =
            (self.active_environment.clone(), result.base_urls.first())
        {
            if let Ok(mut environment) = workspace.load_environment(&name) {
                if !environment.variables.contains_key("base_url") {
                    environment.set("base_url", &candidate.url);
                    let _ = workspace.save_environment(&environment);
                }
            }
        }

        let view = ProjectScan::from_result(&result);
        self.last_scan = Some(result);
        Ok(view)
    }

    /// The most recent scan, if one has been run.
    pub fn last_scan(&self) -> Option<&ScanResult> {
        self.last_scan.as_ref()
    }

    /// Turn a discovered endpoint into an editable request.
    ///
    /// Refuses an endpoint whose path never resolved: sending `/?/stats` would hit a
    /// meaningless URL, and the gap is the thing worth showing rather than papering over.
    pub fn request_for(&self, endpoint_id: &str, base_url: Option<&str>) -> Result<RequestDraft> {
        let scan = self.last_scan.as_ref().ok_or(CoreError::NoScan)?;

        let spec = scan
            .endpoints
            .iter()
            .find(|e| e.id.as_str() == endpoint_id)
            .ok_or_else(|| CoreError::NoSuchEndpoint {
                id: endpoint_id.to_string(),
            })?;

        if !spec.path.is_resolved() {
            return Err(CoreError::UnresolvedEndpoint {
                id: endpoint_id.to_string(),
                expressions: spec
                    .path
                    .unresolved_exprs()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            });
        }

        // `{{base_url}}` by default, so switching environments repoints every request.
        let base = base_url.unwrap_or("{{base_url}}");
        let mut draft = RequestDraft::from_spec(spec, base);
        draft.name = Some(spec.summary.clone().unwrap_or_else(|| spec.display()));
        Ok(draft)
    }

    /// Open a discovered endpoint's definition in the developer's editor.
    ///
    /// The path is resolved against the workspace root and checked to be inside it, because
    /// it reaches here from a scan result and this method starts a process with it.
    pub fn reveal_in_editor(&self, file: &Path, line: u32) -> Result<()> {
        let root = self.workspace()?.layout().root().to_path_buf();
        let full = root.join(file);

        let canonical = full
            .canonicalize()
            .map_err(|_| CoreError::NoSuchSource { path: full.clone() })?;
        if !canonical.starts_with(&root) {
            return Err(CoreError::NoSuchSource { path: full });
        }

        open_in_editor(&canonical, line);
        Ok(())
    }

    // --- sending ------------------------------------------------------------------------

    /// Resolve, send, and record.
    ///
    /// Works without a workspace, in which case there are no variables to resolve and
    /// nothing is written to history.
    pub async fn send(&self, draft: &RequestDraft) -> Result<Exchange> {
        let variables = match self.workspace.as_ref() {
            Some(workspace) => workspace.variable_context(self.active_environment.as_deref())?,
            None => VariableContext::new(),
        };

        // Undefined variables are reported together, before anything is sent, so the user
        // fixes them in one pass rather than one failed request at a time.
        let undefined = draft
            .variable_references()
            .into_iter()
            .filter(|name| !variables.is_defined(name))
            .collect::<Vec<_>>();
        if !undefined.is_empty() {
            return Err(CoreError::UndefinedVariables { names: undefined });
        }

        let (resolved, _secrets_used) = draft.resolve(&variables)?;
        let outcome = self.engine.execute(&resolved).await;

        // History records failures too: "it did not connect" is worth keeping.
        if let Some(workspace) = self.workspace.as_ref() {
            if let Ok(history) = workspace.history() {
                let entry = build_entry(&resolved, &outcome);
                let _ = history.record(&entry.redacted(&variables));
            }
        }

        outcome.map_err(CoreError::from)
    }

    // --- history ------------------------------------------------------------------------

    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        Ok(self.open_history()?.recent(limit)?)
    }

    pub fn history_entry(&self, id: i64) -> Result<Option<HistoryEntry>> {
        Ok(self.open_history()?.get(id)?)
    }

    pub fn clear_history(&self) -> Result<()> {
        Ok(self.open_history()?.clear()?)
    }

    fn open_history(&self) -> Result<History> {
        Ok(self.workspace()?.history()?)
    }

    // --- import -------------------------------------------------------------------------

    pub fn import_curl(&self, text: &str) -> Result<Imported<RequestDraft>> {
        Ok(rl_import::parse_curl(text)?)
    }

    pub fn import_raw_http(&self, text: &str) -> Result<Imported<RequestDraft>> {
        Ok(rl_import::parse_raw_http(text)?)
    }

    pub fn import_openapi(&self, text: &str) -> Result<Imported<OpenApiImport>> {
        Ok(rl_import::parse_openapi(text)?)
    }

    /// Import a specification and save it as a collection.
    ///
    /// Returns the imported document alongside any warnings, so the UI can show what could
    /// not be honoured rather than presenting a silently partial import.
    pub fn import_openapi_as_collection(
        &self,
        text: &str,
        collection_name: &str,
    ) -> Result<Imported<OpenApiImport>> {
        let imported = self.import_openapi(text)?;
        let base_url = imported
            .value
            .servers
            .first()
            .cloned()
            .unwrap_or_else(|| "{{base_url}}".to_string());

        let mut collection = Collection::new(collection_name);
        for spec in &imported.value.endpoints {
            let mut draft = RequestDraft::from_spec(spec, &base_url);
            draft.name = Some(spec.summary.clone().unwrap_or_else(|| spec.display()));
            collection.push(draft);
        }

        self.workspace()?.save_collection(&collection)?;
        Ok(imported)
    }
}

/// Jump to a file and line in whatever editor is available.
///
/// Editors that accept a line number are tried first, because landing on the right line is
/// the whole point; the OS default is the fallback so the action never simply does nothing.
/// Failures are silent by design — a missing editor is not worth an error dialog.
fn open_in_editor(path: &Path, line: u32) {
    let target = format!("{}:{}", path.display(), line);

    for (program, args) in [
        ("code", vec!["-g", target.as_str()]),
        ("cursor", vec!["-g", target.as_str()]),
        ("subl", vec![target.as_str()]),
        ("zed", vec![target.as_str()]),
    ] {
        if std::process::Command::new(program)
            .args(&args)
            .spawn()
            .is_ok()
        {
            return;
        }
    }

    // No line number available from here, but the file still opens.
    let path = path.as_os_str();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn build_entry(sent: &RequestDraft, outcome: &rl_http::Result<Exchange>) -> NewEntry {
    let mut entry = NewEntry::new(sent.method.to_string(), sent.url_with_path_values());

    match outcome {
        Ok(exchange) => {
            entry.status = Some(exchange.response.status);
            entry.duration_ms = Some(exchange.response.timing.total_ms);
            entry.request = serde_json::to_value(&exchange.request).unwrap_or_default();
            entry.response = serde_json::to_value(&exchange.response).ok();
        }
        Err(error) => {
            entry.error = Some(error.to_string());
            entry.request = serde_json::to_value(sent).unwrap_or_default();
        }
    }

    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use rl_model::{AuthConfig, HttpMethod};
    use tempfile::TempDir;

    fn app() -> (TempDir, RouteLens) {
        let dir = TempDir::new().unwrap();
        let mut app = RouteLens::new();
        app.create_workspace(dir.path(), "test", WorkspaceKind::Project)
            .unwrap();
        (dir, app)
    }

    #[test]
    fn a_new_workspace_reports_itself() {
        let (_dir, app) = app();
        let info = app.info().unwrap();
        assert_eq!(info.name, "test");
        assert!(info.collections.is_empty());
        assert!(app.is_open());
    }

    #[test]
    fn operations_without_a_workspace_are_refused_clearly() {
        let app = RouteLens::new();
        assert!(matches!(app.info(), Err(CoreError::NoWorkspace)));
        assert!(matches!(
            app.collection_names(),
            Err(CoreError::NoWorkspace)
        ));
    }

    #[test]
    fn switching_to_an_unknown_environment_is_refused() {
        let (_dir, mut app) = app();
        assert!(app.set_active_environment(Some("nope")).is_err());
        assert_eq!(app.active_environment(), None);
    }

    #[test]
    fn switching_environments_changes_the_variables() {
        let (_dir, mut app) = app();

        let mut local = Environment::new("local");
        local.set("base_url", "http://localhost:8000");
        app.save_environment(&local).unwrap();

        let mut staging = Environment::new("staging");
        staging.set("base_url", "https://staging.example.com");
        app.save_environment(&staging).unwrap();

        app.set_active_environment(Some("local")).unwrap();
        assert_eq!(
            app.variables()
                .unwrap()
                .resolve("{{base_url}}")
                .unwrap()
                .value,
            "http://localhost:8000"
        );

        app.set_active_environment(Some("staging")).unwrap();
        assert_eq!(
            app.variables()
                .unwrap()
                .resolve("{{base_url}}")
                .unwrap()
                .value,
            "https://staging.example.com"
        );
    }

    #[test]
    fn saving_the_same_request_twice_replaces_rather_than_duplicates() {
        let (_dir, app) = app();

        let mut draft = RequestDraft::new(HttpMethod::Get, "https://x.test/a");
        draft.name = Some("Get a".into());
        app.save_request("Users", draft.clone()).unwrap();

        draft.url = "https://x.test/b".into();
        app.save_request("Users", draft).unwrap();

        let collection = app.load_collection("Users").unwrap();
        assert_eq!(collection.len(), 1);
        assert_eq!(collection.requests[0].url, "https://x.test/b");
    }

    #[tokio::test]
    async fn a_request_with_an_undefined_variable_is_refused_before_sending() {
        let (_dir, app) = app();
        let draft = RequestDraft::new(HttpMethod::Get, "{{base_url}}/users");

        let error = app.send(&draft).await.unwrap_err();
        match error {
            CoreError::UndefinedVariables { names } => assert_eq!(names, vec!["base_url"]),
            other => panic!("expected undefined variables, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn every_undefined_variable_is_reported_at_once() {
        let (_dir, app) = app();
        let mut draft = RequestDraft::new(HttpMethod::Get, "{{host}}/users");
        draft
            .headers
            .push(rl_model::KeyValue::new("X-A", "{{token}}"));

        let error = app.send(&draft).await.unwrap_err();
        match error {
            CoreError::UndefinedVariables { names } => {
                assert!(names.contains(&"host".to_string()));
                assert!(names.contains(&"token".to_string()));
            }
            other => panic!("expected undefined variables, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failed_send_is_still_recorded_in_history() {
        let (_dir, app) = app();
        // Nothing listens on port 1.
        let draft = RequestDraft::new(HttpMethod::Get, "http://127.0.0.1:1/");

        assert!(app.send(&draft).await.is_err());

        let history = app.history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].error.is_some());
        assert_eq!(history[0].status, None);
    }

    #[test]
    fn importing_curl_produces_a_sendable_draft() {
        let (_dir, app) = app();
        let imported = app
            .import_curl(
                r#"curl 'https://api.example.com/users?page=2' -H 'Authorization: Bearer t'"#,
            )
            .unwrap();

        assert_eq!(imported.value.url, "https://api.example.com/users");
        assert_eq!(
            imported.value.auth,
            AuthConfig::Bearer { token: "t".into() }
        );
    }

    #[test]
    fn importing_openapi_writes_a_collection_of_requests() {
        let (_dir, app) = app();
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Petstore", "version": "1.0"},
            "servers": [{"url": "https://api.example.com"}],
            "paths": {
                "/pets": {"get": {"summary": "List pets"}, "post": {"summary": "Create pet"}},
                "/pets/{id}": {"get": {"summary": "Get pet"}}
            }
        }"#;

        let imported = app.import_openapi_as_collection(spec, "Petstore").unwrap();
        assert_eq!(imported.value.endpoints.len(), 3);

        let collection = app.load_collection("Petstore").unwrap();
        assert_eq!(collection.len(), 3);

        let list = collection.find("List pets").unwrap();
        assert_eq!(list.url, "https://api.example.com/pets");
        // The discovered spec is linked, so the request knows where it came from.
        assert!(list.spec_ref.is_some());
    }

    #[test]
    fn an_openapi_import_without_a_server_falls_back_to_a_variable() {
        let (_dir, app) = app();
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "X", "version": "1"},
            "paths": {"/x": {"get": {}}}
        }"#;

        let imported = app.import_openapi_as_collection(spec, "X").unwrap();
        assert!(
            !imported.is_clean(),
            "the missing server should be reported"
        );

        let collection = app.load_collection("X").unwrap();
        assert!(collection.requests[0].url.starts_with("{{base_url}}"));
    }

    /// A workspace with a small FastAPI project inside it.
    fn project_workspace() -> (TempDir, RouteLens) {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "fastapi\n").unwrap();
        std::fs::create_dir_all(dir.path().join("app")).unwrap();
        std::fs::write(
            dir.path().join("app/api.py"),
            "from fastapi import APIRouter\n\
             router = APIRouter(prefix=\"/users\", tags=[\"users\"])\n\
             @router.get(\"/{user_id}\")\n\
             def get_user(user_id: int): ...\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("app/main.py"),
            "from fastapi import FastAPI\n\
             from .api import router\n\
             app = FastAPI()\n\
             app.include_router(router, prefix=\"/api/v1\")\n",
        )
        .unwrap();

        let mut app = RouteLens::new();
        app.create_workspace(dir.path(), "fixture", WorkspaceKind::Project)
            .unwrap();
        (dir, app)
    }

    #[test]
    fn scanning_finds_the_projects_endpoints() {
        let (_dir, mut app) = project_workspace();
        let result = app.scan().unwrap();

        assert_eq!(result.frameworks[0].id, "fastapi");
        assert_eq!(result.endpoints.len(), 1);
        assert_eq!(result.endpoints[0].display, "GET /api/v1/users/{user_id}");
        assert!(result.endpoints[0].source.is_some());
    }

    #[test]
    fn a_discovered_endpoint_becomes_an_editable_request() {
        let (_dir, mut app) = project_workspace();
        let scan = app.scan().unwrap();
        let id = scan.endpoints[0].id.clone();

        let draft = app.request_for(&id, None).unwrap();

        // Defaults to the variable, so switching environments repoints it.
        assert_eq!(draft.url, "{{base_url}}/api/v1/users/{user_id}");
        assert_eq!(draft.spec_ref.as_ref().unwrap().as_str(), id);
        assert!(draft.path_values.contains_key("user_id"));
    }

    #[test]
    fn an_endpoint_with_an_unresolved_path_is_refused_rather_than_sent_somewhere_meaningless() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "fastapi\n").unwrap();
        std::fs::write(
            dir.path().join("main.py"),
            "from fastapi import FastAPI, APIRouter\n\
             from .config import settings\n\
             app = FastAPI()\n\
             router = APIRouter()\n\
             @router.get(\"/ping\")\n\
             def ping(): ...\n\
             app.include_router(router, prefix=settings.API_PREFIX)\n",
        )
        .unwrap();

        let mut app = RouteLens::new();
        app.create_workspace(dir.path(), "x", WorkspaceKind::Project)
            .unwrap();
        let scan = app.scan().unwrap();
        let id = scan.endpoints[0].id.clone();

        match app.request_for(&id, None) {
            Err(CoreError::UnresolvedEndpoint { expressions, .. }) => {
                assert_eq!(expressions, vec!["settings.API_PREFIX"]);
            }
            other => panic!("expected a refusal naming the gap, got {other:?}"),
        }
    }

    #[test]
    fn scanning_seeds_an_empty_base_url_but_never_overwrites_a_chosen_one() {
        let (_dir, mut app) = project_workspace();

        let mut env = Environment::new("local");
        app.save_environment(&env).unwrap();
        app.set_active_environment(Some("local")).unwrap();

        app.scan().unwrap();
        let seeded = app.load_environment("local").unwrap();
        assert_eq!(
            seeded.variables.get("base_url").map(String::as_str),
            Some("http://localhost:8000")
        );

        // A deliberate choice survives a rescan.
        env = seeded;
        env.set("base_url", "https://staging.example.com");
        app.save_environment(&env).unwrap();
        app.scan().unwrap();

        assert_eq!(
            app.load_environment("local")
                .unwrap()
                .variables
                .get("base_url")
                .map(String::as_str),
            Some("https://staging.example.com")
        );
    }

    #[test]
    fn a_standalone_workspace_has_nothing_to_scan() {
        let dir = TempDir::new().unwrap();
        let mut app = RouteLens::new();
        app.create_workspace(dir.path(), "client", WorkspaceKind::Standalone)
            .unwrap();

        assert!(matches!(app.scan(), Err(CoreError::NotAProject)));
    }

    #[test]
    fn opening_an_endpoint_before_scanning_is_refused_clearly() {
        let (_dir, app) = project_workspace();
        assert!(matches!(
            app.request_for("GET /x", None),
            Err(CoreError::NoScan)
        ));
    }

    #[test]
    fn secret_names_are_listable_without_exposing_values() {
        let (_dir, app) = app();
        app.set_secret("api_token", "s3cr3t").unwrap();

        assert_eq!(app.secret_names().unwrap(), vec!["api_token"]);
        // And the value is reachable only through resolution.
        let ctx = app.variables().unwrap();
        assert_eq!(ctx.resolve("{{secret:api_token}}").unwrap().value, "s3cr3t");
    }

    #[test]
    fn missing_secrets_are_reported_by_the_workspace_summary() {
        let (_dir, mut app) = app();
        let mut env = Environment::new("local");
        env.set("base_url", "http://x.test")
            .expect_secret("api_token");
        app.save_environment(&env).unwrap();
        app.set_active_environment(Some("local")).unwrap();

        assert_eq!(app.info().unwrap().missing_secrets, vec!["api_token"]);

        app.set_secret("api_token", "s3cr3t").unwrap();
        assert!(app.info().unwrap().missing_secrets.is_empty());
    }
}
