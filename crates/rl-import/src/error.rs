//! Import errors.

pub type Result<T> = std::result::Result<T, ImportError>;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("nothing to import")]
    Empty,

    #[error("no URL found in the command")]
    NoUrl,

    #[error("`{flag}` expects a value")]
    MissingValue { flag: String },

    #[error("{url:?} is not a valid URL")]
    InvalidUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },

    #[error("could not split the command into words")]
    Shell(#[source] crate::shell::ShellError),

    #[error("could not parse the request line: {line:?}")]
    BadRequestLine { line: String },

    #[error("the document is not valid JSON or YAML")]
    NotStructured,

    #[error("this does not look like an OpenAPI or Swagger document")]
    NotOpenApi,

    #[error("unsupported specification version {version:?}")]
    UnsupportedVersion { version: String },

    #[error("could not resolve reference {reference:?}")]
    UnresolvedRef { reference: String },
}
