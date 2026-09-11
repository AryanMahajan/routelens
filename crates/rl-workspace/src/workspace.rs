//! Opening, creating, and reading a workspace.

use crate::collection::Collection;
use crate::environment::Environment;
use crate::error::{Result, WorkspaceError};
use crate::layout::{self, Layout};
use crate::secrets::{FileSecretStore, SecretStore};
use rl_model::VariableContext;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// Attached to a source repository; discovery applies.
    #[default]
    Project,
    /// A general-purpose API client workspace, with no project attached.
    Standalone,
}

/// What is known about the attached project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRef {
    /// Relative to the workspace root, so the file stays valid on another machine.
    #[serde(default = "dot")]
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frameworks: Vec<String>,
    /// `module:attribute` for runtime enrich, once the user has confirmed it.
    ///
    /// Stored so the consent decision and its target are recorded together and can be
    /// revoked by editing one file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_target: Option<String>,
}

fn dot() -> PathBuf {
    PathBuf::from(".")
}

// Hand-written rather than derived: `PathBuf::default()` is the *empty* path, which would
// write `root: ''` into workspace.yaml and read as "nowhere" rather than "here".
impl Default for ProjectRef {
    fn default() -> Self {
        ProjectRef {
            root: dot(),
            frameworks: Vec::new(),
            app_target: None,
        }
    }
}

/// `workspace.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub kind: WorkspaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_environment: Option<String>,
    /// Workspace globals — variables shared across every environment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, String>,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl WorkspaceManifest {
    pub fn new(name: impl Into<String>, kind: WorkspaceKind) -> Self {
        WorkspaceManifest {
            version: CURRENT_VERSION,
            name: name.into(),
            kind,
            project: match kind {
                WorkspaceKind::Project => Some(ProjectRef::default()),
                WorkspaceKind::Standalone => None,
            },
            default_environment: None,
            variables: BTreeMap::new(),
        }
    }
}

/// An open workspace.
#[derive(Debug, Clone)]
pub struct Workspace {
    layout: Layout,
    manifest: WorkspaceManifest,
}

impl Workspace {
    /// Create a workspace, writing the directory skeleton and the `.gitignore`.
    ///
    /// The `.gitignore` is written here — at creation — rather than when someone first
    /// notices a credential in a diff. That ordering is the whole point.
    pub fn create(
        root: impl AsRef<Path>,
        name: impl Into<String>,
        kind: WorkspaceKind,
    ) -> Result<Self> {
        let layout = Layout::new(root.as_ref());
        if layout.exists() {
            return Err(WorkspaceError::AlreadyExists(layout.dir()));
        }

        for dir in [
            layout.dir(),
            layout.collections_dir(),
            layout.environments_dir(),
            layout.local_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .map_err(|e| WorkspaceError::io(format!("creating {}", dir.display()), e))?;
        }

        write_file(&layout.gitignore(), layout::GITIGNORE_CONTENTS)?;

        let workspace = Workspace {
            manifest: WorkspaceManifest::new(name, kind),
            layout,
        };
        workspace.save()?;
        Ok(workspace)
    }

    /// Open an existing workspace.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let layout = Layout::new(root.as_ref());
        let path = layout.manifest();
        if !path.is_file() {
            return Err(WorkspaceError::NotFound(layout.dir()));
        }

        let manifest: WorkspaceManifest = read_yaml(&path)?;
        if manifest.version > CURRENT_VERSION {
            return Err(WorkspaceError::UnsupportedVersion {
                found: manifest.version,
                supported: CURRENT_VERSION,
            });
        }

        // Self-heal: a workspace cloned before the ignore rule existed, or one whose
        // .gitignore was deleted, must not start writing secrets into a tracked tree.
        let gitignore = layout.gitignore();
        if !gitignore.is_file() {
            write_file(&gitignore, layout::GITIGNORE_CONTENTS)?;
        }

        Ok(Workspace { layout, manifest })
    }

    pub fn open_or_create(
        root: impl AsRef<Path>,
        name: impl Into<String>,
        kind: WorkspaceKind,
    ) -> Result<Self> {
        let root = root.as_ref();
        if Layout::new(root).exists() {
            Workspace::open(root)
        } else {
            Workspace::create(root, name, kind)
        }
    }

    pub fn exists(root: impl AsRef<Path>) -> bool {
        Layout::new(root.as_ref()).exists()
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn manifest(&self) -> &WorkspaceManifest {
        &self.manifest
    }

    pub fn manifest_mut(&mut self) -> &mut WorkspaceManifest {
        &mut self.manifest
    }

    pub fn save(&self) -> Result<()> {
        write_yaml(&self.layout.manifest(), &self.manifest, "workspace.yaml")
    }

    // --- collections --------------------------------------------------------------------

    pub fn collection_names(&self) -> Result<Vec<String>> {
        list_names(&self.layout.collections_dir())
    }

    pub fn load_collection(&self, name: &str) -> Result<Collection> {
        let path = self.layout.collection_file(name)?;
        if !path.is_file() {
            return Err(WorkspaceError::NoSuchCollection(name.to_string()));
        }
        read_yaml(&path)
    }

    pub fn save_collection(&self, collection: &Collection) -> Result<()> {
        let path = self.layout.collection_file(&collection.name)?;
        write_yaml(&path, collection, &collection.name)
    }

    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let path = self.layout.collection_file(name)?;
        if !path.is_file() {
            return Err(WorkspaceError::NoSuchCollection(name.to_string()));
        }
        std::fs::remove_file(&path)
            .map_err(|e| WorkspaceError::io(format!("removing {}", path.display()), e))
    }

    // --- environments -------------------------------------------------------------------

    pub fn environment_names(&self) -> Result<Vec<String>> {
        list_names(&self.layout.environments_dir())
    }

    pub fn load_environment(&self, name: &str) -> Result<Environment> {
        let path = self.layout.environment_file(name)?;
        if !path.is_file() {
            return Err(WorkspaceError::NoSuchEnvironment(name.to_string()));
        }
        read_yaml(&path)
    }

    pub fn save_environment(&self, environment: &Environment) -> Result<()> {
        let path = self.layout.environment_file(&environment.name)?;
        write_yaml(&path, environment, &environment.name)
    }

    pub fn delete_environment(&self, name: &str) -> Result<()> {
        let path = self.layout.environment_file(name)?;
        if !path.is_file() {
            return Err(WorkspaceError::NoSuchEnvironment(name.to_string()));
        }
        std::fs::remove_file(&path)
            .map_err(|e| WorkspaceError::io(format!("deleting {}", path.display()), e))
    }

    // --- secrets ------------------------------------------------------------------------

    pub fn secrets(&self) -> FileSecretStore {
        FileSecretStore::new(self.layout.secrets_file())
    }

    // --- history ------------------------------------------------------------------------

    /// Open this workspace's request history, creating the database if needed.
    pub fn history(&self) -> Result<crate::history::History> {
        crate::history::History::open(self.layout.history_db())
    }

    // --- putting it together ------------------------------------------------------------

    /// Assemble the variables a request will resolve against.
    ///
    /// This is where the three tiers meet: globals from the manifest, variables from the
    /// named environment, and values from the private secret store. The result feeds
    /// [`rl_model::RequestDraft::resolve`], so what the UI previews is what gets sent.
    pub fn variable_context(&self, environment: Option<&str>) -> Result<VariableContext> {
        let name = environment
            .map(str::to_string)
            .or_else(|| self.manifest.default_environment.clone());

        let mut ctx = VariableContext::new();
        ctx.globals = self.manifest.variables.clone();

        if let Some(name) = name {
            // A default environment naming a file that no longer exists should not stop the
            // workspace from opening — the user can still send requests with globals alone.
            if let Ok(env) = self.load_environment(&name) {
                ctx.environment = env.variables;
            }
        }

        ctx.secrets = self.secrets().load_all()?;
        Ok(ctx)
    }
}

// --- file helpers -----------------------------------------------------------------------

fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| WorkspaceError::io(format!("reading {}", path.display()), e))?;

    yaml_serde::from_str(&text).map_err(|source| WorkspaceError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn write_yaml<T: Serialize>(path: &Path, value: &T, what: &str) -> Result<()> {
    let text = yaml_serde::to_string(value).map_err(|source| WorkspaceError::Encode {
        what: what.to_string(),
        source,
    })?;
    write_file(path, &text)
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| WorkspaceError::io(format!("creating {}", parent.display()), e))?;
    }
    std::fs::write(path, contents)
        .map_err(|e| WorkspaceError::io(format!("writing {}", path.display()), e))
}

/// The display names of every YAML file in a directory, sorted.
fn list_names(dir: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(WorkspaceError::io(format!("reading {}", dir.display()), e)),
    };

    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| layout::name_from_file(&entry.path()))
        .collect();

    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretStore;
    use rl_model::{AuthConfig, HttpMethod, RequestDraft};
    use tempfile::TempDir;

    fn workspace() -> (TempDir, Workspace) {
        let dir = TempDir::new().unwrap();
        let ws = Workspace::create(dir.path(), "myproject", WorkspaceKind::Project).unwrap();
        (dir, ws)
    }

    #[test]
    fn creation_writes_the_skeleton() {
        let (_dir, ws) = workspace();
        let l = ws.layout();
        assert!(l.manifest().is_file());
        assert!(l.collections_dir().is_dir());
        assert!(l.environments_dir().is_dir());
        assert!(l.local_dir().is_dir());
    }

    #[test]
    fn creation_writes_the_gitignore_before_anything_can_land_in_local() {
        let (_dir, ws) = workspace();
        let text = std::fs::read_to_string(ws.layout().gitignore()).unwrap();
        assert!(text.lines().any(|l| l.trim() == "local/"));
    }

    #[test]
    fn opening_restores_a_deleted_gitignore() {
        let (dir, ws) = workspace();
        std::fs::remove_file(ws.layout().gitignore()).unwrap();

        let reopened = Workspace::open(dir.path()).unwrap();
        assert!(
            reopened.layout().gitignore().is_file(),
            "a workspace must never start writing secrets into a tracked tree"
        );
    }

    #[test]
    fn creating_twice_is_refused_rather_than_overwriting() {
        let (dir, _ws) = workspace();
        assert!(matches!(
            Workspace::create(dir.path(), "other", WorkspaceKind::Project),
            Err(WorkspaceError::AlreadyExists(_))
        ));
    }

    #[test]
    fn opening_a_directory_without_a_workspace_reports_not_found() {
        let dir = TempDir::new().unwrap();
        assert!(matches!(
            Workspace::open(dir.path()),
            Err(WorkspaceError::NotFound(_))
        ));
    }

    #[test]
    fn a_newer_workspace_version_is_refused_with_an_explanation() {
        let (dir, ws) = workspace();
        let path = ws.layout().manifest();
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("version: 1", "version: 999");
        std::fs::write(&path, text).unwrap();

        assert!(matches!(
            Workspace::open(dir.path()),
            Err(WorkspaceError::UnsupportedVersion {
                found: 999,
                supported: 1
            })
        ));
    }

    #[test]
    fn manifest_round_trips() {
        let (dir, mut ws) = workspace();
        ws.manifest_mut()
            .variables
            .insert("org".into(), "acme".into());
        ws.manifest_mut().default_environment = Some("local".into());
        ws.save().unwrap();

        let reopened = Workspace::open(dir.path()).unwrap();
        assert_eq!(reopened.manifest(), ws.manifest());
    }

    #[test]
    fn collections_round_trip_and_list() {
        let (_dir, ws) = workspace();
        let mut users = Collection::new("Users");
        let mut req = RequestDraft::new(HttpMethod::Get, "{{base_url}}/users");
        req.name = Some("List users".into());
        users.push(req);

        ws.save_collection(&users).unwrap();
        ws.save_collection(&Collection::new("Auth")).unwrap();

        assert_eq!(ws.collection_names().unwrap(), vec!["Auth", "Users"]);
        assert_eq!(ws.load_collection("Users").unwrap(), users);
    }

    #[test]
    fn a_missing_collection_is_named_in_the_error() {
        let (_dir, ws) = workspace();
        assert!(matches!(
            ws.load_collection("Nope"),
            Err(WorkspaceError::NoSuchCollection(name)) if name == "Nope"
        ));
    }

    #[test]
    fn listing_an_empty_workspace_yields_nothing_rather_than_failing() {
        let (_dir, ws) = workspace();
        assert!(ws.collection_names().unwrap().is_empty());
        assert!(ws.environment_names().unwrap().is_empty());
    }

    #[test]
    fn a_traversing_collection_name_cannot_escape_the_workspace() {
        let (_dir, ws) = workspace();
        let evil = Collection::new("../../escaped");
        assert!(matches!(
            ws.save_collection(&evil),
            Err(WorkspaceError::InvalidName(_))
        ));
    }

    #[test]
    fn environments_round_trip() {
        let (_dir, ws) = workspace();
        let mut env = Environment::new("local");
        env.set("base_url", "http://localhost:8000")
            .expect_secret("api_token");

        ws.save_environment(&env).unwrap();
        assert_eq!(ws.load_environment("local").unwrap(), env);
        assert_eq!(ws.environment_names().unwrap(), vec!["local"]);
    }

    #[test]
    fn variable_context_layers_globals_environment_and_secrets() {
        let (_dir, mut ws) = workspace();
        ws.manifest_mut()
            .variables
            .insert("org".into(), "acme".into());
        ws.manifest_mut().default_environment = Some("local".into());
        ws.save().unwrap();

        let mut env = Environment::new("local");
        env.set("base_url", "http://localhost:8000");
        ws.save_environment(&env).unwrap();

        ws.secrets().set("api_token", "s3cr3t").unwrap();

        let ctx = ws.variable_context(None).unwrap();
        let resolved = ctx
            .resolve("{{base_url}}/{{org}}?t={{secret:api_token}}")
            .unwrap();

        assert_eq!(resolved.value, "http://localhost:8000/acme?t=s3cr3t");
        assert!(resolved.secrets_used.contains("api_token"));
    }

    #[test]
    fn a_default_environment_that_no_longer_exists_does_not_break_opening() {
        let (_dir, mut ws) = workspace();
        ws.manifest_mut().default_environment = Some("deleted".into());
        ws.save().unwrap();

        let ctx = ws.variable_context(None).unwrap();
        assert!(ctx.environment.is_empty());
    }

    /// The end-to-end promise of the three tiers: a committed collection carries the secret
    /// *reference*, the private tier carries the value, and only resolution brings them
    /// together.
    #[test]
    fn a_secret_value_never_reaches_a_committed_file() {
        let (_dir, ws) = workspace();
        ws.secrets().set("api_token", "s3cr3t").unwrap();

        let mut req = RequestDraft::new(HttpMethod::Get, "{{base_url}}/me");
        req.name = Some("Whoami".into());
        req.auth = AuthConfig::Bearer {
            token: "{{secret:api_token}}".into(),
        };
        let mut collection = Collection::new("Auth");
        collection.push(req.clone());
        ws.save_collection(&collection).unwrap();

        let on_disk =
            std::fs::read_to_string(ws.layout().collection_file("Auth").unwrap()).unwrap();
        assert!(on_disk.contains("secret:api_token"));
        assert!(!on_disk.contains("s3cr3t"));

        let mut env = Environment::new("local");
        env.set("base_url", "https://api.example.com");
        ws.save_environment(&env).unwrap();

        let ctx = ws.variable_context(Some("local")).unwrap();
        let (resolved, secrets_used) = req.resolve(&ctx).unwrap();
        assert_eq!(
            resolved.auth,
            AuthConfig::Bearer {
                token: "s3cr3t".into()
            }
        );
        assert!(secrets_used.contains("api_token"));
    }

    /// Committed files are meant to be read and reviewed by a person, so their shape is part
    /// of the contract rather than an implementation detail.
    #[test]
    fn committed_files_stay_readable() {
        let (_dir, ws) = workspace();

        let mut req = RequestDraft::new(HttpMethod::Get, "{{base_url}}/api/v1/users");
        req.name = Some("List users".into());
        req.query.push(rl_model::KeyValue::new("page", "1"));
        let mut collection = Collection::new("Users");
        collection.push(req);
        ws.save_collection(&collection).unwrap();

        let text = std::fs::read_to_string(ws.layout().collection_file("Users").unwrap()).unwrap();

        // Defaults are omitted rather than written out. A `body: {type: none}` block and a
        // full settings block on every GET turn a one-line change into a noisy diff.
        assert!(!text.contains("type: none"), "empty body should be omitted");
        assert!(
            !text.contains("max_redirects"),
            "default settings should be omitted"
        );
        assert!(
            !text.contains("accept_invalid_certs"),
            "never persisted at all"
        );

        // Block-structured YAML, so nested values sit on their own indented lines and a
        // one-field change shows up as a one-line diff. (Braces alone prove nothing here —
        // `{{base_url}}` is a variable reference, not flow style.)
        assert!(
            text.contains("\n  query:\n"),
            "expected block YAML:\n{text}"
        );
        assert!(
            text.contains("\n  - key: page\n"),
            "expected block YAML:\n{text}"
        );

        // The variable reference survives verbatim; nothing resolved it on the way to disk.
        assert!(text.contains("{{base_url}}"));
    }

    #[test]
    fn a_project_workspace_records_its_root_as_here_not_nowhere() {
        let (_dir, ws) = workspace();
        let text = std::fs::read_to_string(ws.layout().manifest()).unwrap();
        assert!(
            !text.contains("root: ''"),
            "an empty path reads as nowhere:\n{text}"
        );
        assert_eq!(
            ws.manifest().project.as_ref().unwrap().root,
            std::path::PathBuf::from(".")
        );
    }
}
