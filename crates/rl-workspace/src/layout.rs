//! Where everything lives on disk.
//!
//! One type owns every path RouteLens writes, so the three-tier split is enforced in a single
//! place rather than by convention scattered across the crate.

use crate::error::{Result, WorkspaceError};
use std::path::{Path, PathBuf};

pub const DIR_NAME: &str = ".routelens";
pub const MANIFEST_FILE: &str = "workspace.yaml";
pub const COLLECTIONS_DIR: &str = "collections";
pub const ENVIRONMENTS_DIR: &str = "environments";
pub const LOCAL_DIR: &str = "local";
pub const SECRETS_FILE: &str = "secrets.json";
pub const HISTORY_DB: &str = "history.sqlite";
pub const INDEX_DB: &str = "index.sqlite";
pub const GITIGNORE_FILE: &str = ".gitignore";

/// Written when a workspace is created, not after someone notices a token in a diff.
///
/// Git-friendly storage plus bearer tokens is exactly how credentials reach version control,
/// so the private tier is excluded from the moment it can exist.
pub const GITIGNORE_CONTENTS: &str = "\
# Written automatically by RouteLens.
#
# The private and disposable storage tiers live in local/: secret values,
# request history, and the rebuildable source index. None of it belongs in
# version control.
#
# The files beside this one -- workspace.yaml, collections/, environments/ --
# are meant to be committed. They contain secret *names* only, never values.
local/
";

/// Every path in a workspace, derived from the directory that contains `.routelens/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// `root` is the directory *containing* `.routelens/` — a project root, or an
    /// application data directory for a standalone workspace.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Layout { root: root.into() }
    }

    /// The directory the workspace lives in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // --- shared tier: committed ---------------------------------------------------------

    pub fn dir(&self) -> PathBuf {
        self.root.join(DIR_NAME)
    }

    pub fn manifest(&self) -> PathBuf {
        self.dir().join(MANIFEST_FILE)
    }

    pub fn collections_dir(&self) -> PathBuf {
        self.dir().join(COLLECTIONS_DIR)
    }

    pub fn environments_dir(&self) -> PathBuf {
        self.dir().join(ENVIRONMENTS_DIR)
    }

    pub fn gitignore(&self) -> PathBuf {
        self.dir().join(GITIGNORE_FILE)
    }

    /// One file per collection, so two people adding requests to different collections do
    /// not conflict.
    pub fn collection_file(&self, name: &str) -> Result<PathBuf> {
        Ok(self.collections_dir().join(file_stem(name)?))
    }

    pub fn environment_file(&self, name: &str) -> Result<PathBuf> {
        Ok(self.environments_dir().join(file_stem(name)?))
    }

    // --- private and disposable tiers: never committed ----------------------------------

    pub fn local_dir(&self) -> PathBuf {
        self.dir().join(LOCAL_DIR)
    }

    /// Private tier. Real credentials — not disposable, not committable.
    pub fn secrets_file(&self) -> PathBuf {
        self.local_dir().join(SECRETS_FILE)
    }

    /// Private tier: history rows may quote request and response bodies.
    pub fn history_db(&self) -> PathBuf {
        self.local_dir().join(HISTORY_DB)
    }

    /// Disposable tier: a cache of parsed source, safe to delete at any time.
    pub fn index_db(&self) -> PathBuf {
        self.local_dir().join(INDEX_DB)
    }

    pub fn exists(&self) -> bool {
        self.manifest().is_file()
    }
}

/// Turn a display name into a file name, rejecting anything that could escape the directory.
///
/// Collection and environment names reach here from user input and from imported documents,
/// so `../../.ssh/id_rsa` has to be refused rather than sanitized into something surprising.
fn file_stem(name: &str) -> Result<String> {
    let trimmed = name.trim();

    let invalid = trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
        || trimmed.contains('\0');

    if invalid {
        return Err(WorkspaceError::InvalidName(name.to_string()));
    }

    Ok(format!("{trimmed}.yaml"))
}

/// The display name a workspace file corresponds to.
pub fn name_from_file(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("yaml") | Some("yml") => Some(stem.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new("/projects/myapp")
    }

    #[test]
    fn shared_tier_sits_directly_under_the_workspace_dir() {
        let l = layout();
        assert!(l.manifest().ends_with(".routelens/workspace.yaml"));
        assert!(l.collections_dir().ends_with(".routelens/collections"));
        assert!(l.environments_dir().ends_with(".routelens/environments"));
    }

    #[test]
    fn private_and_disposable_tiers_sit_under_local() {
        let l = layout();
        for path in [l.secrets_file(), l.history_db(), l.index_db()] {
            assert!(
                path.starts_with(l.local_dir()),
                "{} must live under local/ so the written .gitignore covers it",
                path.display()
            );
        }
    }

    #[test]
    fn the_written_gitignore_covers_the_local_directory() {
        assert!(GITIGNORE_CONTENTS.lines().any(|l| l.trim() == "local/"));
    }

    #[test]
    fn names_become_yaml_files() {
        let l = layout();
        assert!(l.collection_file("Users").unwrap().ends_with("Users.yaml"));
        assert!(l
            .environment_file(" local ")
            .unwrap()
            .ends_with("local.yaml"));
    }

    #[test]
    fn traversal_in_a_name_is_refused_not_sanitized() {
        let l = layout();
        for bad in ["../secrets", "..", ".", "", "a/b", "a\\b", "C:evil"] {
            assert!(
                l.collection_file(bad).is_err(),
                "{bad:?} should have been refused"
            );
        }
    }

    #[test]
    fn file_names_map_back_to_display_names() {
        assert_eq!(
            name_from_file(Path::new("/x/.routelens/collections/Users.yaml")),
            Some("Users".to_string())
        );
        assert_eq!(name_from_file(Path::new("/x/notes.txt")), None);
    }
}
