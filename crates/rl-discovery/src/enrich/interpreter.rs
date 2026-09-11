//! Which Python to run the helper with.
//!
//! The helper imports the project, so it has to run where the project's dependencies are
//! installed: the project's own virtual environment first, then whatever is active in the
//! shell, then whatever is on `PATH`. Every candidate says where it came from, because the
//! consent dialog shows the exact interpreter and the developer should be able to tell a
//! `.venv` from a system Python at a glance.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interpreter {
    pub path: PathBuf,
    /// Why this one, in words the UI can show.
    pub source: String,
}

/// Directory names a virtual environment is usually created under.
const VENV_DIRS: &[&str] = &[".venv", "venv", "env", ".env", "virtualenv"];

/// Collect interpreter candidates, best first.
pub fn detect(project_root: &Path) -> Vec<Interpreter> {
    let mut found: Vec<Interpreter> = Vec::new();
    let mut add = |path: PathBuf, source: String| {
        if path.is_file() && !found.iter().any(|i| i.path == path) {
            found.push(Interpreter { path, source });
        }
    };

    for name in VENV_DIRS {
        let dir = project_root.join(name);
        if let Some(python) = python_in(&dir) {
            add(
                python,
                format!("the project's `{name}` virtual environment"),
            );
        }
    }

    // A `.python-version` or Poetry setup often keeps the environment elsewhere, but an
    // activated one announces itself.
    if let Some(active) = std::env::var_os("VIRTUAL_ENV") {
        if let Some(python) = python_in(Path::new(&active)) {
            add(
                python,
                "the active virtual environment (VIRTUAL_ENV)".to_string(),
            );
        }
    }
    if let Some(conda) = std::env::var_os("CONDA_PREFIX") {
        if let Some(python) = python_in(Path::new(&conda)) {
            add(
                python,
                "the active conda environment (CONDA_PREFIX)".to_string(),
            );
        }
    }

    for name in ["python3", "python"] {
        if let Some(python) = on_path(name) {
            add(python, format!("`{name}` on PATH"));
        }
    }

    found
}

/// The interpreter inside a virtual environment, on either platform's layout.
fn python_in(env: &Path) -> Option<PathBuf> {
    let candidates = if cfg!(windows) {
        vec![
            env.join("Scripts").join("python.exe"),
            env.join("python.exe"),
        ]
    } else {
        vec![
            env.join("bin").join("python3"),
            env.join("bin").join("python"),
        ]
    };
    candidates.into_iter().find(|p| p.is_file())
}

/// Resolve a command name against `PATH` without running it.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(&path)
        .filter(|dir| {
            // The Microsoft Store's `python.exe` stub opens the Store instead of running
            // anything; it is never the answer.
            !dir.to_string_lossy().contains("WindowsApps")
        })
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_project_venv_is_preferred_and_described() {
        let dir = tempfile::TempDir::new().unwrap();
        let python = if cfg!(windows) {
            dir.path().join(".venv/Scripts/python.exe")
        } else {
            dir.path().join(".venv/bin/python")
        };
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(&python, "").unwrap();

        let found = detect(dir.path());
        assert_eq!(found[0].path, python);
        assert!(found[0].source.contains(".venv"));
    }

    #[test]
    fn a_project_without_an_environment_still_offers_something_or_nothing_honestly() {
        let dir = tempfile::TempDir::new().unwrap();
        let found = detect(dir.path());
        // Every candidate must exist on disk; that is the only promise made here.
        assert!(found.iter().all(|i| i.path.is_file()));
    }
}
