use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid name '{0}': use lowercase letters, digits and '-' only")]
    InvalidName(String),
    #[error("'{0}' already exists")]
    AlreadyExists(String),
    #[error("no group '{0}'")]
    NoGroup(String),
    #[error("no folder or group at {0}")]
    NoEntry(String),
    #[error("{} is listed there already", .0.display())]
    FolderListed(PathBuf),
    #[error("workspace '{0}' not found")]
    NotFound(String),
    #[error("not a directory: {}", .0.display())]
    NotADirectory(PathBuf),
    #[error("workspace '{workspace}' has no folder '{folder}'; known: {known}")]
    NoFolder { workspace: String, folder: String, known: String },
    #[error("workspace '{workspace}' has no schedule '{id}'")]
    NoSchedule { workspace: String, id: String },
    #[error("schedule '{id}': {why}")]
    InvalidSchedule { id: String, why: String },
    /// A slash command or prompt file a schedule names is not in its folder.
    #[error("{0}")]
    NoPrompt(String),
    #[error("{0}")]
    Secret(String),
    #[error("Task Scheduler: {0}")]
    Tasks(String),
    #[error("cannot locate the Supraman home: set SUPRAMAN_HOME or APPDATA")]
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
    #[error("cannot serialize the schedules of workspace '{workspace}': {source}")]
    SerializeSchedules {
        workspace: String,
        source: toml::ser::Error,
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
                | Self::NoGroup(_)
                | Self::NoEntry(_)
                | Self::FolderListed(_)
                | Self::NotFound(_)
                | Self::NotADirectory(_)
                | Self::NoFolder { .. }
                | Self::NoSchedule { .. }
                | Self::InvalidSchedule { .. }
                | Self::NoPrompt(_)
        )
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
