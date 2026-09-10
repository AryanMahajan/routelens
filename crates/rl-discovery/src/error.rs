//! Discovery errors.

use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, DiscoveryError>;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),

    #[error("{context}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not walk the project tree")]
    Walk {
        #[source]
        source: ignore::Error,
    },

    #[error("could not load the {language} grammar")]
    Grammar {
        language: &'static str,
        #[source]
        source: tree_sitter::LanguageError,
    },

    #[error("could not build a query for {language}")]
    Query {
        language: &'static str,
        #[source]
        source: tree_sitter::QueryError,
    },

    #[error("index cache error")]
    Cache {
        #[source]
        source: rusqlite::Error,
    },
}

impl DiscoveryError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        DiscoveryError::Io {
            context: context.into(),
            source,
        }
    }
}
