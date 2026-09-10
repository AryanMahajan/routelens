//! Workspace errors.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, WorkspaceError>;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("no RouteLens workspace at {}", .0.display())]
    NotFound(PathBuf),

    #[error("a RouteLens workspace already exists at {}", .0.display())]
    AlreadyExists(PathBuf),

    #[error("no collection named {0:?}")]
    NoSuchCollection(String),

    #[error("no environment named {0:?}")]
    NoSuchEnvironment(String),

    #[error(
        "workspace version {found} was written by a newer RouteLens; this build understands \
         version {supported}"
    )]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("invalid name {0:?}: must not be empty, `.`, `..`, or contain a path separator")]
    InvalidName(String),

    #[error("{context}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not parse {}", .path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: yaml_serde::Error,
    },

    #[error("could not write {what}")]
    Encode {
        what: String,
        #[source]
        source: yaml_serde::Error,
    },

    #[error("could not read the secret store")]
    Secrets {
        #[source]
        source: serde_json::Error,
    },
}

impl WorkspaceError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        WorkspaceError::Io {
            context: context.into(),
            source,
        }
    }
}
