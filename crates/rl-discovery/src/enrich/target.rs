//! Which application object to import.
//!
//! Runtime enrich needs a `module:attribute` target — the same thing `uvicorn` takes. The
//! project usually states it already: a Procfile, a Dockerfile `CMD`, a `FLASK_APP` line.
//! Failing that, the static scan found where `app = FastAPI()` lives, which is the same
//! answer in a different spelling. Candidates are offered ranked; nothing is chosen
//! silently, because importing the wrong module can run the wrong code.

use crate::project::ProjectContext;
use crate::scan::AppRoot;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A candidate application object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTarget {
    /// `app.main:app`, or `app:create_app()` for a factory. What the helper is handed.
    pub target: String,
    /// Where the interpreter must run for the module path to resolve, relative to the
    /// project root. `.` unless the project uses a `src/` layout.
    pub cwd: PathBuf,
    /// Why this was suggested, in words the consent dialog can show.
    pub source: String,
    /// Higher is a better guess.
    pub confidence: u8,
}

/// Servers whose first non-flag argument is a `module:app` target.
const ASGI_SERVERS: &[&str] = &[
    "uvicorn",
    "gunicorn",
    "hypercorn",
    "daphne",
    "waitress-serve",
];

/// Collect target candidates, best first.
pub fn infer(project: &ProjectContext, roots: &[AppRoot]) -> Vec<AppTarget> {
    let mut found: Vec<AppTarget> = Vec::new();
    let mut add = |target: AppTarget| {
        if !found
            .iter()
            .any(|t| t.target == target.target && t.cwd == target.cwd)
        {
            found.push(target);
        }
    };

    for name in [
        "Procfile",
        "Makefile",
        "Dockerfile",
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "pyproject.toml",
        "package.json",
    ] {
        let Some(text) = project.manifest(name) else {
            continue;
        };
        for target in targets_in_commands(text) {
            add(AppTarget {
                target,
                cwd: PathBuf::from("."),
                source: format!("run command in {name}"),
                confidence: 9,
            });
        }
    }

    for name in [
        ".env",
        ".env.local",
        ".env.example",
        "Dockerfile",
        "docker-compose.yml",
    ] {
        let Some(text) = project.manifest(name) else {
            continue;
        };
        for target in flask_app_settings(text) {
            add(AppTarget {
                target,
                cwd: PathBuf::from("."),
                source: format!("FLASK_APP in {name}"),
                confidence: 8,
            });
        }
    }

    // `manage.py` names the settings module Django is configured from.
    if let Some(text) = project.manifest("manage.py") {
        for module in django_settings_modules(text) {
            add(AppTarget {
                target: module,
                cwd: PathBuf::from("."),
                source: "DJANGO_SETTINGS_MODULE in manage.py".to_string(),
                confidence: 9,
            });
        }
    }

    // What the static scan saw: `app = FastAPI()` in `app/main.py` is `app.main:app`.
    let single = roots.len() == 1;
    for root in roots {
        let Some((cwd, module)) = module_of(&root.module, project.files()) else {
            continue;
        };
        let target = match &root.factory {
            Some(factory) => format!("{module}:{factory}()"),
            // Django's root is the settings module itself; the helper builds the resolver.
            None if root.framework == "django" => module,
            None => format!("{module}:{}", root.name),
        };
        add(AppTarget {
            target,
            cwd,
            source: format!(
                "`{}` declared in {}",
                root.name,
                crate::project::display(&root.module)
            ),
            confidence: if single { 7 } else { 5 },
        });
    }

    found.sort_by_key(|t| std::cmp::Reverse(t.confidence));
    found
}

/// `uvicorn app.main:app --reload` → `app.main:app`; `--factory` appends `()`.
fn targets_in_commands(text: &str) -> Vec<String> {
    let mut targets = Vec::new();

    for line in text.lines() {
        let tokens: Vec<&str> = line
            .split_whitespace()
            .map(|t| t.trim_matches(['"', '\'', ',', '[', ']']))
            .collect();

        for (index, token) in tokens.iter().enumerate() {
            let server = token.rsplit(['/', '\\']).next().unwrap_or(token);
            if !ASGI_SERVERS.contains(&server) {
                continue;
            }

            let rest = &tokens[index + 1..];
            let factory = rest.contains(&"--factory");
            let target = rest.iter().enumerate().find_map(|(i, t)| {
                // The target is the first argument that is not a flag or a flag's value.
                if t.starts_with('-') || i > 0 && takes_value(rest[i - 1]) {
                    return None;
                }
                looks_like_target(t).then(|| t.to_string())
            });
            if let Some(mut target) = target {
                if factory && !target.ends_with("()") {
                    target.push_str("()");
                }
                targets.push(target);
            }
        }

        // `flask --app app.main:create_app run`
        if let Some(position) = tokens.iter().position(|t| *t == "--app") {
            if let Some(value) = tokens.get(position + 1) {
                if let Some(target) = flask_app_target(value) {
                    targets.push(target);
                }
            }
        }
        for token in &tokens {
            if let Some(value) = token.strip_prefix("--app=") {
                if let Some(target) = flask_app_target(value) {
                    targets.push(target);
                }
            }
        }
    }

    targets
}

fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "--host"
            | "--port"
            | "-p"
            | "--bind"
            | "-b"
            | "--workers"
            | "-w"
            | "-k"
            | "--worker-class"
            | "--log-level"
            | "--app-dir"
            | "--timeout"
            | "-t"
            | "--env-file"
            | "--root-path"
            | "--ssl-keyfile"
            | "--ssl-certfile"
            | "-c"
            | "--config"
    )
}

/// `app.main:app` and `app:create_app()` are targets; `0.0.0.0:8000` is not.
fn looks_like_target(token: &str) -> bool {
    let Some((module, attribute)) = token.split_once(':') else {
        return false;
    };
    let is_module = !module.is_empty()
        && module
            .split('.')
            .all(|part| !part.is_empty() && is_identifier(part));
    let attribute = attribute.trim_end_matches("()");
    is_module && is_identifier(attribute)
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// `FLASK_APP=app.main:create_app` lines in an env file or a Dockerfile `ENV`.
fn flask_app_settings(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let line = line
                .strip_prefix("ENV ")
                .or_else(|| line.strip_prefix("export "))
                .unwrap_or(line)
                .trim();
            let value = line.strip_prefix("FLASK_APP=")?;
            flask_app_target(value.trim().trim_matches(['"', '\'']))
        })
        .collect()
}

/// Flask's `--app` spellings: `app`, `app.main`, `app:create_app`, `app:create_app()`,
/// `app.py`. A bare module is passed through; the helper applies Flask's own lookup rules.
fn flask_app_target(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let module = value.split(':').next()?;
    let module = module
        .strip_suffix(".py")
        .unwrap_or(module)
        .replace(['/', '\\'], ".");
    if !module.split('.').all(is_identifier) {
        return None;
    }
    let attribute = value.split_once(':').map(|(_, a)| a);
    Some(match attribute {
        Some(attribute) => {
            // Flask lets `create_app("config")` carry arguments; the helper cannot pass
            // them, so the call is left to its own no-argument form.
            let name = attribute.split('(').next().unwrap_or(attribute);
            if attribute.contains('(') {
                format!("{module}:{name}()")
            } else {
                format!("{module}:{name}")
            }
        }
        None => module,
    })
}

/// `os.environ.setdefault("DJANGO_SETTINGS_MODULE", "config.settings")` in `manage.py`.
fn django_settings_modules(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.contains("DJANGO_SETTINGS_MODULE"))
        .filter_map(|line| {
            let after = line.split("DJANGO_SETTINGS_MODULE").nth(1)?;
            // The first quoted piece after the key that reads as a module path; the
            // key's own closing quote makes counting quotes unreliable.
            after
                .split(['"', '\''])
                .map(str::trim)
                .find(|value| !value.is_empty() && value.split('.').all(is_identifier))
                .map(str::to_string)
        })
        .collect()
}

/// The dotted module a file is imported as, and the directory to import it from.
///
/// Walks up while `__init__.py` exists: `app/api/users.py` under `app/__init__.py` is
/// `app.api.users` from the root; `src/pkg/main.py` with no `src/__init__.py` is
/// `pkg.main` from `src/`.
fn module_of(file: &Path, project_files: &[PathBuf]) -> Option<(PathBuf, String)> {
    let stem = file.file_stem()?.to_str()?;
    let mut parts: Vec<String> = Vec::new();
    if stem != "__init__" {
        parts.push(stem.to_string());
    }

    let has_init = |dir: &Path| project_files.iter().any(|f| f == &dir.join("__init__.py"));

    let mut dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    while !dir.as_os_str().is_empty() && has_init(&dir) {
        parts.push(dir.file_name()?.to_str()?.to_string());
        dir = dir.parent().map(Path::to_path_buf).unwrap_or_default();
    }

    parts.reverse();
    if parts.is_empty() {
        return None;
    }
    let cwd = if dir.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        dir
    };
    Some((cwd, parts.join(".")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_uvicorn_command_names_the_target() {
        assert_eq!(
            targets_in_commands("web: uvicorn app.main:app --host 0.0.0.0 --port 9000\n"),
            vec!["app.main:app"]
        );
    }

    #[test]
    fn flags_before_the_target_are_skipped_and_factory_is_marked() {
        assert_eq!(
            targets_in_commands("uvicorn --port 8000 --factory app:create_app\n"),
            vec!["app:create_app()"]
        );
        assert_eq!(
            targets_in_commands("gunicorn -w 4 -k uvicorn.workers.UvicornWorker main:app\n"),
            vec!["main:app"]
        );
    }

    #[test]
    fn a_dockerfile_cmd_array_is_read() {
        assert_eq!(
            targets_in_commands("CMD [\"uvicorn\", \"app.main:app\", \"--port\", \"80\"]\n"),
            vec!["app.main:app"]
        );
    }

    #[test]
    fn a_bind_address_is_not_mistaken_for_a_target() {
        assert!(targets_in_commands("gunicorn --bind 0.0.0.0:8000\n").is_empty());
    }

    #[test]
    fn flask_app_spellings() {
        assert_eq!(flask_app_target("app"), Some("app".into()));
        assert_eq!(flask_app_target("app.py"), Some("app".into()));
        assert_eq!(
            flask_app_target("app:create_app"),
            Some("app:create_app".into())
        );
        assert_eq!(
            flask_app_target("app:create_app('dev')"),
            Some("app:create_app()".into())
        );
        assert_eq!(
            targets_in_commands("web: flask --app app.main:create_app run\n"),
            vec!["app.main:create_app"]
        );
        assert_eq!(
            flask_app_settings("# comment\nFLASK_APP=app\nFLASK_ENV=dev\n"),
            vec!["app"]
        );
        assert_eq!(
            flask_app_settings("ENV FLASK_APP=\"app:create_app()\"\n"),
            vec!["app:create_app()"]
        );
    }

    #[test]
    fn manage_py_names_the_django_settings_module() {
        assert_eq!(
            django_settings_modules(
                "def main():\n    os.environ.setdefault(\"DJANGO_SETTINGS_MODULE\", \"config.settings\")\n"
            ),
            vec!["config.settings"]
        );
    }

    #[test]
    fn module_paths_follow_init_files() {
        let tree = files(&[
            "app/__init__.py",
            "app/main.py",
            "app/api/__init__.py",
            "app/api/users.py",
            "main.py",
            "src/pkg/__init__.py",
            "src/pkg/main.py",
        ]);
        assert_eq!(
            module_of(Path::new("app/main.py"), &tree),
            Some((PathBuf::from("."), "app.main".into()))
        );
        assert_eq!(
            module_of(Path::new("app/__init__.py"), &tree),
            Some((PathBuf::from("."), "app".into()))
        );
        assert_eq!(
            module_of(Path::new("app/api/users.py"), &tree),
            Some((PathBuf::from("."), "app.api.users".into()))
        );
        assert_eq!(
            module_of(Path::new("main.py"), &tree),
            Some((PathBuf::from("."), "main".into()))
        );
        assert_eq!(
            module_of(Path::new("src/pkg/main.py"), &tree),
            Some((PathBuf::from("src"), "pkg.main".into()))
        );
    }

    #[test]
    fn app_roots_become_targets_and_a_factory_is_called() {
        use std::fs;
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("app")).unwrap();
        fs::write(dir.path().join("app/__init__.py"), "").unwrap();
        fs::write(dir.path().join("Procfile"), "web: uvicorn app.main:app\n").unwrap();
        let project = ProjectContext::scan(dir.path()).unwrap();

        let roots = vec![AppRoot {
            framework: "flask".into(),
            module: PathBuf::from("app/__init__.py"),
            name: "app".into(),
            factory: Some("create_app".into()),
        }];
        let targets = infer(&project, &roots);
        let spelled: Vec<&str> = targets.iter().map(|t| t.target.as_str()).collect();
        assert_eq!(spelled, vec!["app.main:app", "app:create_app()"]);
        assert!(targets[0].confidence > targets[1].confidence);
    }
}
