use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid workspace name '{0}': use lowercase letters, digits and '-' only")]
    InvalidName(String),
    #[error("workspace '{0}' already exists (use --force to replace it)")]
    AlreadyExists(String),
    #[error("workspace '{0}' not found")]
    NotFound(String),
    #[error("not a directory: {}", .0.display())]
    NotADirectory(PathBuf),
    #[error("workspace '{workspace}' has no folder '{folder}'; known: {known}")]
    NoFolder { workspace: String, folder: String, known: String },
    #[error("workspace '{workspace}' has no scan of {}", path.display())]
    NoScan { workspace: String, path: PathBuf },
    #[error("{folder} is supervised in workspace {owner}; a folder's session belongs to one workspace")]
    SupervisedElsewhere { folder: String, owner: String },
    #[error("cannot locate the Foreman home: set FOREMAN_HOME or APPDATA")]
    NoHome,
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot serialize workspace '{name}': {source}")]
    Serialize {
        name: String,
        source: toml::ser::Error,
    },
}

impl CoreError {
    /// True when the caller asked for something wrong; false when the disk or environment failed.
    pub fn is_user_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidName(_)
                | Self::AlreadyExists(_)
                | Self::NotFound(_)
                | Self::NotADirectory(_)
                | Self::NoFolder { .. }
                | Self::NoScan { .. }
                | Self::SupervisedElsewhere { .. }
        )
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
