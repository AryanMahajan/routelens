//! Errors surfaced to a shell.

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no workspace is open")]
    NoWorkspace,

    /// Reported together rather than one failed request at a time, so the user fixes every
    /// missing variable in one pass.
    #[error("undefined variable(s): {}", .names.join(", "))]
    UndefinedVariables { names: Vec<String> },

    #[error(transparent)]
    Workspace(#[from] rl_workspace::WorkspaceError),

    #[error(transparent)]
    Http(#[from] rl_http::HttpError),

    #[error(transparent)]
    Import(#[from] rl_import::ImportError),

    #[error(transparent)]
    Resolve(#[from] rl_model::ResolveError),
}

impl CoreError {
    /// A message fit to put in front of a user, with the source chain flattened.
    ///
    /// `thiserror` prints only the outermost message by default, which strips exactly the
    /// detail that makes an error actionable — "request failed" rather than "connection
    /// refused".
    pub fn message(&self) -> String {
        let mut parts = vec![self.to_string()];
        let mut source = std::error::Error::source(self);
        while let Some(current) = source {
            let text = current.to_string();
            if !parts.contains(&text) {
                parts.push(text);
            }
            source = current.source();
        }
        parts.join(": ")
    }
}
