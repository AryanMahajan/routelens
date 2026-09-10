//! The private tier.
//!
//! Secret **values** live here and nowhere else. Committed files carry secret *names* only,
//! referenced from requests as `{{secret:name}}`.

use crate::error::{Result, WorkspaceError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Read and write secret values.
///
/// A trait because the file backend is the fallback, not the goal — the OS keychain
/// (Credential Manager, Keychain, Secret Service) is the better home and slots in behind
/// this interface without anything above it changing.
pub trait SecretStore {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&mut self, name: &str, value: &str) -> Result<()>;
    fn delete(&mut self, name: &str) -> Result<()>;
    /// Names only. Useful for showing what a workspace expects without reading values.
    fn names(&self) -> Result<Vec<String>>;
    /// Every secret, for building a [`rl_model::VariableContext`].
    fn load_all(&self) -> Result<BTreeMap<String, String>>;
}

/// JSON file backend, living in the workspace's `local/` directory.
///
/// Beyond filesystem permissions this offers no protection: treat the file with the same care
/// as a `.env`. `docs/security.md` states that limitation rather than implying more.
#[derive(Debug, Clone)]
pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileSecretStore { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) if text.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(text) => {
                serde_json::from_str(&text).map_err(|source| WorkspaceError::Secrets { source })
            }
            // A missing store is an empty store, not an error: a fresh clone has no secrets.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(source) => Err(WorkspaceError::io(
                format!("reading {}", self.path.display()),
                source,
            )),
        }
    }

    fn write(&self, secrets: &BTreeMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| WorkspaceError::io(format!("creating {}", parent.display()), e))?;
        }

        let json = serde_json::to_string_pretty(secrets)
            .map_err(|source| WorkspaceError::Secrets { source })?;

        std::fs::write(&self.path, json)
            .map_err(|e| WorkspaceError::io(format!("writing {}", self.path.display()), e))?;

        self.restrict_permissions()?;
        Ok(())
    }

    /// Owner-only, where the platform expresses that through file modes.
    ///
    /// On Windows the equivalent is an ACL change, which is left to the OS default rather
    /// than attempted here — claiming a protection that is not actually applied would be
    /// worse than documenting its absence.
    fn restrict_permissions(&self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&self.path, perms)
                .map_err(|e| WorkspaceError::io(format!("securing {}", self.path.display()), e))?;
        }
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.read()?.get(name).cloned())
    }

    fn set(&mut self, name: &str, value: &str) -> Result<()> {
        let mut secrets = self.read()?;
        secrets.insert(name.to_string(), value.to_string());
        self.write(&secrets)
    }

    fn delete(&mut self, name: &str) -> Result<()> {
        let mut secrets = self.read()?;
        secrets.remove(name);
        self.write(&secrets)
    }

    fn names(&self) -> Result<Vec<String>> {
        Ok(self.read()?.into_keys().collect())
    }

    fn load_all(&self) -> Result<BTreeMap<String, String>> {
        self.read()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, FileSecretStore) {
        let dir = TempDir::new().unwrap();
        let store = FileSecretStore::new(dir.path().join("local").join("secrets.json"));
        (dir, store)
    }

    #[test]
    fn a_missing_store_reads_as_empty() {
        let (_dir, store) = store();
        assert_eq!(store.load_all().unwrap(), BTreeMap::new());
        assert_eq!(store.get("anything").unwrap(), None);
    }

    #[test]
    fn values_round_trip() {
        let (_dir, mut store) = store();
        store.set("api_token", "s3cr3t").unwrap();
        assert_eq!(store.get("api_token").unwrap(), Some("s3cr3t".into()));
    }

    #[test]
    fn writing_creates_the_local_directory() {
        let (_dir, mut store) = store();
        store.set("a", "1").unwrap();
        assert!(store.path().is_file());
    }

    #[test]
    fn deleting_removes_only_the_named_secret() {
        let (_dir, mut store) = store();
        store.set("a", "1").unwrap();
        store.set("b", "2").unwrap();
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(store.get("b").unwrap(), Some("2".into()));
    }

    #[test]
    fn names_are_listable_without_reading_values() {
        let (_dir, mut store) = store();
        store.set("api_token", "s3cr3t").unwrap();
        store.set("db_password", "hunter2").unwrap();
        assert_eq!(store.names().unwrap(), vec!["api_token", "db_password"]);
    }

    #[test]
    fn an_empty_file_is_treated_as_an_empty_store() {
        let (_dir, store) = store();
        std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        std::fs::write(store.path(), "   \n").unwrap();
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn feeds_a_variable_context() {
        let (_dir, mut store) = store();
        store.set("api_token", "s3cr3t").unwrap();

        let ctx = rl_model::VariableContext::new().with_secrets(store.load_all().unwrap());
        let resolved = ctx.resolve("Bearer {{secret:api_token}}").unwrap();

        assert_eq!(resolved.value, "Bearer s3cr3t");
        assert!(resolved.secrets_used.contains("api_token"));
    }
}
