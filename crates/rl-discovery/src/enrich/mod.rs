//! Runtime enrich: ask the application instead of reading it.
//!
//! Everything else in this crate is static. This module is the one place project code is
//! executed, and it is built so that cannot happen by accident:
//!
//! 1. [`plan`] only *proposes* — a target, an interpreter, the exact command line. It runs
//!    nothing and writes nothing.
//! 2. [`run`] executes precisely the plan it is handed. The caller is expected to have shown
//!    that plan and been told yes; see `docs/discovery/runtime-enrich.md`.
//! 3. The helper script is written to disk before it runs, so what is shown is what runs
//!    and the developer can read it.
//! 4. Output is JSON on stdout; the caller feeds it through the OpenAPI importer and
//!    [`merge`] unions it with the static result. Static locations are never lost.
//!
//! ```text
//! plan(project, roots) ─▶ EnrichPlan ─▶ (consent) ─▶ run(plan) ─▶ document
//!                                                                    │
//!                          static endpoints ─────▶ merge ◀── OpenAPI importer
//! ```

pub mod interpreter;
pub mod merge;
pub mod target;

pub use interpreter::Interpreter;
pub use merge::{MergeReport, Provenance, PROVENANCE_KEY};
pub use target::AppTarget;

use crate::project::ProjectContext;
use crate::scan::AppRoot;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The helper, embedded so a build of RouteLens carries exactly one version of it.
pub const HELPER_SOURCE: &str = include_str!("helper.py");
/// What the helper is called on disk. Written under `.routelens/local/`.
pub const HELPER_FILE_NAME: &str = "routelens_enrich.py";
/// How long the application gets to import before the run is abandoned.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum EnrichError {
    #[error(
        "no Python interpreter was found; a project virtual environment (`.venv`) is the usual fix"
    )]
    NoInterpreter,

    #[error("could not tell which module holds the application; name it as `module:attribute`")]
    NoTarget,

    #[error("could not write the helper script to {}", path.display())]
    WriteHelper {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not start `{command}`")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the helper did not finish within {} seconds; the application may be waiting on something at import time", .0.as_secs())]
    Timeout(Duration),

    #[error("the helper exited with {status}:\n{stderr}")]
    Failed { status: String, stderr: String },

    #[error("the helper's output was not the expected JSON: {reason}")]
    BadOutput { reason: String },
}

/// Everything about a proposed run, in a form the consent dialog can show verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrichPlan {
    pub interpreter: Interpreter,
    pub target: AppTarget,
    /// Absolute. The helper runs here so the module path resolves.
    pub cwd: PathBuf,
    /// Absolute. Where the helper is written before it runs.
    pub helper_path: PathBuf,
}

impl EnrichPlan {
    /// The command line, spelled for a human. This is what the developer approves.
    pub fn command_line(&self) -> String {
        format!(
            "cd {} && {} {} {}",
            quote(&self.cwd.display().to_string()),
            quote(&self.interpreter.path.display().to_string()),
            quote(&self.helper_path.display().to_string()),
            quote(&self.target.target)
        )
    }
}

fn quote(text: &str) -> String {
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

/// What a plan is chosen from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidates {
    pub targets: Vec<AppTarget>,
    pub interpreters: Vec<Interpreter>,
}

/// Work out what could be run. Executes nothing.
pub fn candidates(project: &ProjectContext, roots: &[AppRoot]) -> Candidates {
    Candidates {
        targets: target::infer(project, roots),
        interpreters: interpreter::detect(project.root()),
    }
}

/// Assemble a plan from a chosen target and interpreter.
///
/// `helper_dir` is where the script will be written — the workspace's gitignored local
/// directory, so the file is inspectable but never committed.
pub fn plan(
    project_root: &Path,
    helper_dir: &Path,
    interpreter: Interpreter,
    target: AppTarget,
) -> EnrichPlan {
    let cwd = if target.cwd.as_os_str() == "." {
        project_root.to_path_buf()
    } else {
        project_root.join(&target.cwd)
    };
    EnrichPlan {
        interpreter,
        target,
        cwd,
        helper_path: helper_dir.join(HELPER_FILE_NAME),
    }
}

/// What a successful run produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrichOutput {
    /// `fastapi` or `flask`, as the helper identified it.
    pub framework: String,
    /// An OpenAPI 3 document, ready for the importer.
    pub openapi: serde_json::Value,
    /// Whatever the application printed while importing. Shown, never parsed.
    pub stderr: String,
    pub duration: Duration,
}

/// Execute a plan. **This runs the project's code.** Only call it with a plan the developer
/// has seen and approved.
pub fn run(plan: &EnrichPlan, timeout: Duration) -> Result<EnrichOutput, EnrichError> {
    if let Some(dir) = plan.helper_path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| EnrichError::WriteHelper {
            path: plan.helper_path.clone(),
            source,
        })?;
    }
    std::fs::write(&plan.helper_path, HELPER_SOURCE).map_err(|source| {
        EnrichError::WriteHelper {
            path: plan.helper_path.clone(),
            source,
        }
    })?;

    let started = Instant::now();
    let mut child = Command::new(&plan.interpreter.path)
        .arg(&plan.helper_path)
        .arg(&plan.target.target)
        .current_dir(&plan.cwd)
        // Importing must not litter the project with bytecode caches.
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("ROUTELENS_ENRICH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| EnrichError::Spawn {
            command: plan.command_line(),
            source,
        })?;

    // Drain both pipes on threads: an application that logs a lot at import time would
    // otherwise fill one pipe and deadlock against our wait on the other.
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let out_thread = std::thread::spawn(move || read_all(stdout));
    let err_thread = std::thread::spawn(move || read_all(stderr));

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EnrichError::Timeout(timeout));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(source) => {
                return Err(EnrichError::Spawn {
                    command: plan.command_line(),
                    source,
                })
            }
        }
    };

    let stdout = out_thread.join().unwrap_or_default();
    let stderr = err_thread.join().unwrap_or_default();
    let duration = started.elapsed();

    if !status.success() {
        return Err(EnrichError::Failed {
            status: status
                .code()
                .map(|c| format!("status {c}"))
                .unwrap_or_else(|| "a signal".to_string()),
            stderr: tail(&stderr, 40),
        });
    }

    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|e| EnrichError::BadOutput {
            reason: e.to_string(),
        })?;
    let framework = value
        .get("framework")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| EnrichError::BadOutput {
            reason: "no `framework` field".into(),
        })?
        .to_string();
    let openapi = value
        .get("openapi")
        .cloned()
        .filter(serde_json::Value::is_object)
        .ok_or_else(|| EnrichError::BadOutput {
            reason: "no `openapi` document".into(),
        })?;

    Ok(EnrichOutput {
        framework,
        openapi,
        stderr,
        duration,
    })
}

fn read_all(mut pipe: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The last `lines` lines — a traceback's useful end, not a wall of import logging.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_for(dir: &Path, interpreter: PathBuf, target: &str) -> EnrichPlan {
        plan(
            dir,
            &dir.join(".routelens").join("local"),
            Interpreter {
                path: interpreter,
                source: "test".into(),
            },
            AppTarget {
                target: target.into(),
                cwd: PathBuf::from("."),
                source: "test".into(),
                confidence: 9,
            },
        )
    }

    #[test]
    fn the_command_line_shows_every_part_of_the_plan() {
        let dir = tempfile::TempDir::new().unwrap();
        let plan = plan_for(
            dir.path(),
            PathBuf::from("/venv/bin/python"),
            "app.main:app",
        );
        let line = plan.command_line();
        assert!(line.contains("/venv/bin/python"));
        assert!(line.contains(HELPER_FILE_NAME));
        assert!(line.ends_with("app.main:app"));
    }

    #[test]
    fn a_missing_interpreter_is_a_spawn_error_not_a_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        let plan = plan_for(
            dir.path(),
            dir.path().join("definitely-not-python"),
            "app:app",
        );
        let error = run(&plan, DEFAULT_TIMEOUT).unwrap_err();
        assert!(matches!(error, EnrichError::Spawn { .. }), "{error}");
        assert!(plan.helper_path.is_file(), "the helper is written first");
    }

    /// End to end against the Flask fixture, when a Python with Flask is available.
    /// Skipped otherwise, loudly enough to notice.
    #[test]
    fn the_flask_fixture_reports_its_url_map() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/flask")
            .canonicalize()
            .unwrap();
        let Some(python) = python_with("flask") else {
            eprintln!("skipping: no Python with flask importable (set ROUTELENS_TEST_PYTHON)");
            return;
        };
        let scratch = tempfile::TempDir::new().unwrap();
        let mut plan = plan_for(&root, python, "app:create_app()");
        plan.helper_path = scratch.path().join(HELPER_FILE_NAME);

        let output = run(&plan, DEFAULT_TIMEOUT).unwrap();
        assert_eq!(output.framework, "flask");
        let paths = output.openapi["paths"].as_object().unwrap();
        assert!(
            paths.contains_key("/dyn/widgets"),
            "loop-registered routes are exact at runtime"
        );
        assert!(
            paths.contains_key("/admin/stats"),
            "the config prefix is known at runtime"
        );
        assert!(
            !paths.contains_key("/static/{filename}"),
            "Flask's static route is dropped"
        );
    }

    #[test]
    fn the_fastapi_fixture_reports_its_openapi_document() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/fastapi")
            .canonicalize()
            .unwrap();
        let Some(python) = python_with("fastapi") else {
            eprintln!("skipping: no Python with fastapi importable (set ROUTELENS_TEST_PYTHON)");
            return;
        };
        let scratch = tempfile::TempDir::new().unwrap();
        let mut plan = plan_for(&root, python, "app.main:app");
        plan.helper_path = scratch.path().join(HELPER_FILE_NAME);

        let output = run(&plan, DEFAULT_TIMEOUT).unwrap();
        assert_eq!(output.framework, "fastapi");
        let create = &output.openapi["paths"]["/api/v1/users/"]["post"];
        assert!(
            create["requestBody"]["content"]["application/json"]["schema"].is_object(),
            "the real Pydantic schema arrives, which static analysis could only name"
        );
        assert!(output.openapi["paths"]["/admin/stats"].is_object());
    }

    #[test]
    fn a_broken_target_fails_with_the_traceback() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/flask")
            .canonicalize()
            .unwrap();
        let Some(python) = python_with("flask") else {
            return;
        };
        let scratch = tempfile::TempDir::new().unwrap();
        let mut plan = plan_for(&root, python, "nope:app");
        plan.helper_path = scratch.path().join(HELPER_FILE_NAME);

        let error = run(&plan, DEFAULT_TIMEOUT).unwrap_err();
        match error {
            EnrichError::Failed { stderr, .. } => {
                assert!(stderr.contains("No module named 'nope'"))
            }
            other => panic!("expected a helper failure, got {other}"),
        }
    }

    /// `ROUTELENS_TEST_PYTHON`, or the first detected interpreter that can import `module`.
    fn python_with(module: &str) -> Option<PathBuf> {
        let candidates: Vec<PathBuf> = std::env::var_os("ROUTELENS_TEST_PYTHON")
            .map(|p| vec![PathBuf::from(p)])
            .unwrap_or_else(|| {
                interpreter::detect(Path::new("."))
                    .into_iter()
                    .map(|i| i.path)
                    .collect()
            });
        candidates.into_iter().find(|python| {
            Command::new(python)
                .args(["-c", &format!("import {module}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }
}
